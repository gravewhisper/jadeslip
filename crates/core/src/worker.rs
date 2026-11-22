use crate::replace::Replacer;
use crate::search::{Case, Hit, Outcome, POLL, Run, SearchEnd, counted_hit, hit_at};
use crate::{ChapterIx, Limits, Reader};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const BOOK_FD: i32 = 3;
const WORKER_NICE: i32 = 10;
const COMPILE_DEADLINE: Duration = Duration::from_secs(5);
const LOAD_DEADLINE: Duration = Duration::from_secs(15);
const MAX_FRAME_BYTES: u64 = 8 << 20;
const FLUSH_HITS: usize = 256;
const FLUSH_AFTER: Duration = Duration::from_millis(20);
const MAX_REPLACED_BYTES: usize = 1 << 20;
const START_FAILED: &str = "Regex search could not start, so nothing was searched.";

pub trait PatternEngine {
    type Compiled;
    fn compile(pattern: &str, case: Case) -> Result<Self::Compiled, String>;
    fn check_template(compiled: &Self::Compiled, template: &str) -> Result<(), String>;
    fn matches(
        compiled: &Self::Compiled,
        text: &str,
        template: Option<&str>,
        emit: &mut dyn FnMut(usize, usize, Option<String>) -> bool,
    ) -> Result<(), String>;
}

#[derive(Serialize, Deserialize)]
enum Request {
    Compile {
        pattern: String,
        case: Case,
        limits: Limits,
    },
    Chapter {
        ix: u32,
        max_hits: u32,
        delay_ms: u64,
        template: Option<String>,
        row_limit: u32,
    },
    Probe {
        bytes: u64,
    },
}

#[derive(Serialize, Deserialize)]
enum Reply {
    Ready,
    BadPattern(String),
    BadTemplate(String),
    Loaded,
    Hits(Vec<Hit>),
    Done,
    Limited,
    Unreadable(String),
    Probed { refused: bool },
}

fn send<T: Serialize>(out: &mut impl Write, message: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *out, message)?;
    out.write_all(b"\n")?;
    out.flush()
}

fn read_frame<T: for<'de> Deserialize<'de>>(input: &mut impl BufRead) -> Option<T> {
    let mut line = Vec::new();
    let read = input
        .by_ref()
        .take(MAX_FRAME_BYTES)
        .read_until(b'\n', &mut line)
        .ok()?;
    if read == 0 || line.last() != Some(&b'\n') {
        return None;
    }
    serde_json::from_slice(&line).ok()
}

fn exit_on_parent_close() -> Receiver<Request> {
    let (requests, incoming) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        while let Some(request) = read_frame(&mut input) {
            if requests.send(request).is_err() {
                break;
            }
        }
        unsafe { libc::_exit(0) };
    });
    incoming
}

struct ChapterFrames<'a, W: Write> {
    out: &'a mut W,
    pending: Vec<Hit>,
    flushed_at: Instant,
    found: u32,
    max_hits: u32,
    row_limit: u32,
    oversized: bool,
}

impl<W: Write> ChapterFrames<'_, W> {
    fn flush(&mut self) -> io::Result<()> {
        self.flushed_at = Instant::now();
        if self.pending.is_empty() {
            return Ok(());
        }
        send(self.out, &Reply::Hits(std::mem::take(&mut self.pending)))
    }

    fn wants_row(&self) -> bool {
        self.found < self.row_limit
    }

    fn push(&mut self, hit: Hit) -> bool {
        if hit.replaced.as_ref().map_or(0, String::len) > MAX_REPLACED_BYTES {
            self.oversized = true;
            return false;
        }
        self.pending.push(hit);
        self.found += 1;
        if (self.pending.len() >= FLUSH_HITS || self.flushed_at.elapsed() >= FLUSH_AFTER)
            && self.flush().is_err()
        {
            return false;
        }
        self.found < self.max_hits
    }
}

struct ChapterJob {
    ix: u32,
    max_hits: u32,
    delay_ms: u64,
    template: Option<String>,
    row_limit: u32,
}

fn run_chapter<E: PatternEngine>(
    reader: &Reader<File>,
    compiled: &E::Compiled,
    job: &ChapterJob,
    out: &mut impl Write,
) -> io::Result<()> {
    let template = job.template.as_deref();
    if let Some(Err(message)) = template.map(|template| E::check_template(compiled, template)) {
        return send(out, &Reply::BadTemplate(message));
    }
    let visible = match reader.visible_text(ChapterIx(job.ix)) {
        Ok(visible) => visible,
        Err(error) => return send(out, &Reply::Unreadable(error.user_message())),
    };
    send(out, &Reply::Loaded)?;
    if job.delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(job.delay_ms));
    }
    let mut frames = ChapterFrames {
        out,
        pending: Vec::new(),
        flushed_at: Instant::now(),
        found: 0,
        max_hits: job.max_hits,
        row_limit: job.row_limit,
        oversized: false,
    };
    let text = visible.as_str();
    let finished = if text.is_empty() {
        Ok(())
    } else {
        E::matches(compiled, text, template, &mut |start, end, replaced| {
            let mut hit = if frames.wants_row() {
                hit_at(&visible, start, end)
            } else {
                counted_hit(&visible, start, end)
            };
            hit.same = replaced.as_deref() == Some(&text[start..end]);
            hit.replaced = replaced;
            frames.push(hit)
        })
    };
    let oversized = frames.oversized;
    frames.flush()?;
    send(
        out,
        &match finished {
            Ok(()) if !oversized => Reply::Done,
            _ => Reply::Limited,
        },
    )
}

fn worker_main<E: PatternEngine>() -> Option<()> {
    let requests = exit_on_parent_close();
    let mut out = BufWriter::new(io::stdout());
    let mut compiled = None;
    let mut reader = None;
    while let Ok(request) = requests.recv() {
        match request {
            Request::Compile {
                pattern,
                case,
                limits,
            } => match E::compile(&pattern, case) {
                Ok(ready) => {
                    let file = unsafe { File::from_raw_fd(BOOK_FD) };
                    reader = Some(Reader::open(file, limits).ok()?);
                    compiled = Some(ready);
                    send(&mut out, &Reply::Ready).ok()?;
                }
                Err(message) => send(&mut out, &Reply::BadPattern(message)).ok()?,
            },
            Request::Chapter {
                ix,
                max_hits,
                delay_ms,
                template,
                row_limit,
            } => {
                let job = ChapterJob {
                    ix,
                    max_hits,
                    delay_ms,
                    template,
                    row_limit,
                };
                run_chapter::<E>(reader.as_ref()?, compiled.as_ref()?, &job, &mut out).ok()?;
            }
            Request::Probe { bytes } => {
                let refused = Vec::<u8>::new().try_reserve_exact(bytes as usize).is_err();
                send(&mut out, &Reply::Probed { refused }).ok()?;
            }
        }
    }
    Some(())
}

pub fn run_worker<E: PatternEngine>() -> ! {
    let outcome = worker_main::<E>();
    std::process::exit(if outcome.is_some() { 0 } else { 1 })
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<Reply>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn harden_child(book_fd: i32, parent: libc::pid_t, memory_cap: u64) -> io::Result<()> {
    let check = |result: libc::c_int| {
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    };
    unsafe {
        #[cfg(target_os = "linux")]
        check(libc::prctl(
            libc::PR_SET_PDEATHSIG,
            libc::SIGKILL as libc::c_ulong,
        ))?;
        if libc::getppid() != parent {
            libc::_exit(1);
        }
        if book_fd == BOOK_FD {
            check(libc::fcntl(BOOK_FD, libc::F_SETFD, 0))?;
        } else {
            check(libc::dup2(book_fd, BOOK_FD))?;
        }
        #[cfg(target_os = "linux")]
        libc::close_range(
            BOOK_FD as libc::c_uint + 1,
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC as libc::c_int,
        );
        let memory = libc::rlimit {
            rlim_cur: memory_cap,
            rlim_max: memory_cap,
        };
        check(libc::setrlimit(libc::RLIMIT_AS, &memory))?;
        let no_core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        check(libc::setrlimit(libc::RLIMIT_CORE, &no_core))?;
        libc::setpriority(libc::PRIO_PROCESS, 0, WORKER_NICE);
    }
    Ok(())
}

impl Worker {
    fn spawn(exe: &Path, reader: &Reader<File>, memory_cap: u64) -> io::Result<Worker> {
        let book_fd = reader.source().as_raw_fd();
        let parent = unsafe { libc::getpid() };
        let mut command = Command::new(exe);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || harden_child(book_fd, parent, memory_cap));
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().ok_or(io::ErrorKind::BrokenPipe)?;
        let stdout = child.stdout.take().ok_or(io::ErrorKind::BrokenPipe)?;
        let (replies_in, replies) = mpsc::channel();
        std::thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            while let Some(reply) = read_frame(&mut input) {
                if replies_in.send(reply).is_err() {
                    break;
                }
            }
        });
        Ok(Worker {
            child,
            stdin,
            replies,
        })
    }

    fn request(&mut self, request: &Request) -> io::Result<()> {
        send(&mut self.stdin, request)
    }

    fn compile(
        &mut self,
        pattern: &str,
        case: Case,
        limits: Limits,
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), Started> {
        self.request(&Request::Compile {
            pattern: pattern.to_owned(),
            case,
            limits,
        })
        .map_err(|_| Started::Died)?;
        let deadline = Instant::now() + COMPILE_DEADLINE;
        while Instant::now() < deadline {
            if stopped() {
                return Err(Started::Stopped);
            }
            match self.replies.recv_timeout(POLL) {
                Ok(Reply::Ready) => return Ok(()),
                Ok(Reply::BadPattern(message)) => return Err(Started::Bad(message)),
                Ok(_) | Err(RecvTimeoutError::Disconnected) => return Err(Started::Died),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        Err(Started::Died)
    }
}

enum Started {
    Bad(String),
    Died,
    NoWorker,
    Stopped,
}

fn delay_of(run: &Run, chapter: ChapterIx) -> u64 {
    run.options
        .delay
        .as_ref()
        .map_or(0, |delay| delay(chapter).as_millis() as u64)
}

fn wait_for_chapter(worker: &mut Worker, run: &Run, chapter: ChapterIx) -> (Outcome, bool) {
    let mut hits = Vec::new();
    let scan = run.scan.as_deref();
    let template = scan.and_then(|scan| match &scan.replacer {
        Replacer::Template(template) => Some(template.clone()),
        Replacer::Literal { .. } | Replacer::Remove => None,
    });
    let sent = worker.request(&Request::Chapter {
        ix: chapter.0,
        max_hits: run.hits_wanted(),
        delay_ms: delay_of(run, chapter),
        template,
        row_limit: scan.map_or(u32::MAX, |scan| scan.row_limit),
    });
    if sent.is_err() {
        return (Outcome::Limited(hits), false);
    }
    let mut deadline = Instant::now() + LOAD_DEADLINE;
    loop {
        if run.is_stopped() {
            return (Outcome::Limited(hits), false);
        }
        let now = Instant::now();
        if now >= deadline {
            return (Outcome::Limited(hits), false);
        }
        match worker.replies.recv_timeout(POLL.min(deadline - now)) {
            Ok(Reply::Loaded) => deadline = Instant::now() + run.deadline_for(chapter),
            Ok(Reply::Hits(more)) => hits.extend(more),
            Ok(Reply::Done) => return (Outcome::Hits(hits), true),
            Ok(Reply::Limited) => return (Outcome::Limited(hits), true),
            Ok(Reply::Unreadable(message)) => return (Outcome::Unreadable(message), true),
            Ok(Reply::BadTemplate(message)) => {
                run.end_early(SearchEnd::BadPattern(message));
                return (Outcome::Limited(hits), true);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Ok(_) | Err(RecvTimeoutError::Disconnected) => {
                return (Outcome::Limited(hits), false);
            }
        }
    }
}

struct Pattern {
    exe: Arc<Path>,
    text: Arc<str>,
    case: Case,
}

fn start_worker(run: &Run, pattern: &Pattern) -> Result<Worker, Started> {
    let mut worker = Worker::spawn(&pattern.exe, &run.reader, run.options.memory_cap)
        .map_err(|_| Started::NoWorker)?;
    worker.compile(&pattern.text, pattern.case, run.reader.limits(), &|| {
        run.is_stopped()
    })?;
    Ok(worker)
}

fn supervise(run: &Run, pattern: &Pattern, mut worker: Option<Worker>) {
    while let Some((position, chapter)) = run.take() {
        let Some(permit) = run.permit_for(chapter) else {
            return;
        };
        if worker.is_none() {
            worker = start_worker(run, pattern).ok();
        }
        let Some(running) = worker.as_mut() else {
            run.put(position, Outcome::Limited(Vec::new()));
            continue;
        };
        let (outcome, keep) = wait_for_chapter(running, run, chapter);
        drop(permit);
        if !keep {
            worker = None;
        }
        run.put(position, outcome);
    }
}

pub(crate) fn start_pool(run: &Arc<Run>, pattern: String, case: Case) {
    let Some(exe) = run.options.worker_exe.clone() else {
        run.end_early(SearchEnd::Failed(START_FAILED.to_owned()));
        return;
    };
    let pattern = Arc::new(Pattern {
        exe: exe.into(),
        text: pattern.into(),
        case,
    });
    let run = run.clone();
    std::thread::spawn(move || {
        let first = match start_worker(&run, &pattern) {
            Ok(first) => first,
            Err(Started::Bad(message)) => return run.end_early(SearchEnd::BadPattern(message)),
            Err(Started::Stopped) => return,
            Err(Started::NoWorker) => {
                return run.end_early(SearchEnd::Failed(START_FAILED.to_owned()));
            }
            Err(Started::Died) => {
                return run.end_early(SearchEnd::BadPattern(
                    "That pattern is too complex to run.".to_owned(),
                ));
            }
        };
        for _ in 1..run.options.threads.max(1) {
            let run = run.clone();
            let pattern = pattern.clone();
            std::thread::spawn(move || supervise(&run, &pattern, None));
        }
        supervise(&run, &pattern, Some(first));
    });
}

#[doc(hidden)]
pub fn probe_worker(
    exe: &Path,
    reader: &Reader<File>,
    memory_cap: u64,
    bytes: u64,
) -> io::Result<(bool, u32)> {
    let mut worker = Worker::spawn(exe, reader, memory_cap)?;
    worker.request(&Request::Probe { bytes })?;
    match worker.replies.recv_timeout(Duration::from_secs(5)) {
        Ok(Reply::Probed { refused }) => Ok((refused, worker.child.id())),
        _ => Err(io::ErrorKind::BrokenPipe.into()),
    }
}
