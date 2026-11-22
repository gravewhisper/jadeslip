use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const KEY_RIGHT: u32 = 0xff53;
pub const KEY_LEFT: u32 = 0xff51;
pub const KEY_ESCAPE: u32 = 0xff1b;
pub const KEY_ENTER: u32 = 0xff0d;
pub const KEY_BACKSPACE: u32 = 0xff08;
pub const KEY_UP: u32 = 0xff52;
pub const KEY_DOWN: u32 = 0xff54;
pub const KEY_PAGE_DOWN: u32 = 0xff56;
pub const KEY_F11: u32 = 0xffc8;
pub const KEY_SPACE: u32 = 0x20;
pub const MASK_SHIFT: u32 = 1;
pub const MASK_CTRL: u32 = 4;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repo root exists")
}

pub fn generated_book(name: &str) -> PathBuf {
    repo_root()
        .join("fixtures/generated")
        .join(format!("{name}.epub"))
}

pub fn real_books() -> Vec<PathBuf> {
    let mut books: Vec<PathBuf> = std::fs::read_dir(repo_root().join("fixtures/real"))
        .expect("fixtures/real exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "epub"))
        .collect();
    books.sort();
    books
}

pub fn book_label(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    stem.rsplit('-').next().unwrap_or_default().to_owned()
}

pub fn largest_real_book() -> PathBuf {
    real_books()
        .into_iter()
        .max_by_key(|path| {
            let file = std::fs::File::open(path).expect("real book opens");
            jadeslip_core::Reader::open(file, jadeslip_core::DEFAULT_LIMITS)
                .map_or(0, |reader| reader.rows().len())
        })
        .expect("fixtures/real has a book")
}

pub fn scratch_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().expect("own path");
        let stem = exe.file_stem().expect("a test binary name").to_string_lossy();
        let suite = stem.rsplit_once('-').map_or(&*stem, |(suite, _)| suite);
        let dir = repo_root().join("target/e2e-run").join(suite);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    })
    .clone()
}

pub fn wall_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs_f64()
        * 1e3
}

pub fn e2e_binary() -> PathBuf {
    let path = repo_root().join("target/e2e/release/jadeslip");
    assert!(
        path.exists(),
        "build it first: tests/e2e/run.sh ({})",
        path.display()
    );
    path
}

pub fn plain_release_binary() -> PathBuf {
    repo_root().join("target/release/jadeslip")
}

pub fn headless_output() -> String {
    std::env::var("SS_HEADLESS_OUTPUT").expect("GUI tests only run through tests/e2e/headless.sh")
}

fn hyprctl_json(args: &[&str]) -> Value {
    let output = Command::new("hyprctl")
        .args(args)
        .arg("-j")
        .output()
        .expect("hyprctl runs");
    serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
}

pub fn window_monitor_of(pid: u32) -> Option<String> {
    let clients = hyprctl_json(&["clients"]);
    let client = clients
        .as_array()?
        .iter()
        .find(|client| client["pid"].as_u64() == Some(u64::from(pid)))?;
    let monitor = client["monitor"].as_i64()?;
    let monitors = hyprctl_json(&["monitors"]);
    monitors
        .as_array()?
        .iter()
        .find(|entry| entry["id"].as_i64() == Some(monitor))
        .and_then(|entry| entry["name"].as_str().map(str::to_owned))
}

pub fn window_fullscreen_of(pid: u32) -> Option<u64> {
    let clients = hyprctl_json(&["clients"]);
    let client = clients
        .as_array()?
        .iter()
        .find(|client| client["pid"].as_u64() == Some(u64::from(pid)))?;
    let state = &client["fullscreen"];
    state.as_u64().or_else(|| state.as_bool().map(u64::from))
}

fn descendants(root: u32) -> Vec<u32> {
    let mut parents = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, tail)) = stat.rsplit_once(')') else {
            continue;
        };
        if let Some(parent) = tail
            .split_whitespace()
            .nth(1)
            .and_then(|text| text.parse::<u32>().ok())
        {
            parents.push((pid, parent));
        }
    }
    let mut tree = vec![root];
    let mut at = 0;
    while at < tree.len() {
        let parent = tree[at];
        tree.extend(
            parents
                .iter()
                .filter(|(_, of)| *of == parent)
                .map(|(pid, _)| *pid),
        );
        at += 1;
    }
    tree
}

fn status_kb(text: &str, key: &str) -> u64 {
    text.lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Memory {
    pub rss_kb: u64,
    pub pss_kb: u64,
    pub processes: u32,
}

pub fn memory_of_tree(root: u32) -> Memory {
    let mut total = Memory::default();
    for pid in descendants(root) {
        let Ok(rollup) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) else {
            continue;
        };
        total.rss_kb += status_kb(&rollup, "Rss:");
        total.pss_kb += status_kb(&rollup, "Pss:");
        total.processes += 1;
    }
    total
}

static NEXT_APP: AtomicU32 = AtomicU32::new(0);

pub struct App {
    child: Child,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    socket: PathBuf,
    data: PathBuf,
    keep_data: bool,
    pub spawned_at_ms: f64,
    pub pid: u32,
}

impl App {
    pub fn launch(book: Option<&Path>, off: &[&str]) -> App {
        let mut command = Command::new(e2e_binary());
        if let Some(book) = book {
            command.arg(book);
        }
        App::spawn(command, off)
    }

    pub fn spawn(command: Command, off: &[&str]) -> App {
        let id = NEXT_APP.load(Ordering::SeqCst);
        let data = scratch_dir().join(format!("data-{}-{id}", std::process::id()));
        let mut app = App::spawn_with_data(command, off, data);
        app.keep_data = false;
        app
    }

    pub fn launch_with_data_opt(book: Option<&Path>, data: &Path) -> App {
        let mut command = Command::new(e2e_binary());
        if let Some(book) = book {
            command.arg(book);
        }
        App::spawn_with_data(command, &[], data.to_path_buf())
    }

    pub fn launch_with_data_env(book: Option<&Path>, data: &Path, env: &[(&str, &Path)]) -> App {
        let mut command = Command::new(e2e_binary());
        if let Some(book) = book {
            command.arg(book);
        }
        for (name, value) in env {
            command.env(name, value);
        }
        App::spawn_with_data(command, &[], data.to_path_buf())
    }

    pub fn launch_binary(binary: &Path, book: Option<&Path>, data: &Path) -> App {
        let mut command = Command::new(binary);
        if let Some(book) = book {
            command.arg(book);
        }
        App::spawn_with_data(command, &[], data.to_path_buf())
    }

    pub fn launch_with_data(book: &Path, data: &Path) -> App {
        let mut command = Command::new(e2e_binary());
        command.arg(book);
        App::spawn_with_data(command, &[], data.to_path_buf())
    }

    fn spawn_with_data(mut command: Command, off: &[&str], data: PathBuf) -> App {
        headless_output();
        let id = NEXT_APP.fetch_add(1, Ordering::SeqCst);
        let scratch = scratch_dir();
        let sockets =
            std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let socket = sockets.join(format!("jadeslip-ctl-{}-{id}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let log = std::fs::File::create(scratch.join("app.stderr.log")).expect("log file");
        let temp = scratch.join("tmp");
        std::fs::create_dir_all(&temp).expect("app temp folder");
        command
            .env("TMPDIR", &temp)
            .env("JADESLIP_E2E_SOCKET", &socket)
            .env("JADESLIP_DATA_DIR", &data)
            .env("JADESLIP_E2E_OFF", off.join(","))
            .stdout(Stdio::null())
            .stderr(log);
        let spawned_at_ms = wall_ms();
        let child = command.spawn().expect("app starts");
        let pid = child.id();
        let stream = connect(&socket);
        stream
            .set_read_timeout(Some(Duration::from_secs(180)))
            .expect("socket timeout");
        let writer = stream.try_clone().expect("socket clone");
        App {
            child,
            reader: BufReader::new(stream),
            writer,
            socket,
            data,
            keep_data: true,
            spawned_at_ms,
            pid,
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn request_close_through_the_compositor(&self) {
        let script = format!(
            "hl.dispatch(hl.dsp.window.close({{ window = 'pid:{}' }}))",
            self.pid
        );
        let done = Command::new("hyprctl")
            .args(["eval", &script])
            .output()
            .expect("hyprctl runs");
        let reply = String::from_utf8_lossy(&done.stdout);
        assert_eq!(reply.trim(), "ok", "the compositor refused the close");
    }

    pub fn assert_on_headless_output(&self) {
        let wanted = headless_output();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match window_monitor_of(self.pid) {
                Some(monitor) if monitor == wanted => return,
                Some(monitor) => panic!("the test window opened on {monitor}, not {wanted}"),
                None if Instant::now() > deadline => panic!("no test window appeared"),
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    pub fn op(&mut self, command: &Value) -> Value {
        writeln!(self.writer, "{command}").expect("control socket write");
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .expect("control socket read");
        serde_json::from_str(&line).unwrap_or(Value::Null)
    }

    pub fn try_js(&mut self, script: &str, timeout_ms: u64) -> Result<Value, String> {
        let reply = self.op(&json!({"op": "eval", "js": script, "timeout": timeout_ms}));
        if reply["ok"].as_bool() == Some(true) {
            Ok(reply.get("v").cloned().unwrap_or(Value::Null))
        } else {
            Err(reply["err"].as_str().unwrap_or("no reply").to_owned())
        }
    }

    pub fn js(&mut self, script: &str) -> Value {
        self.try_js(script, 60_000)
            .unwrap_or_else(|error| panic!("page script failed: {error}"))
    }

    pub fn click(&mut self, x: f64, y: f64) {
        self.op(&json!({"op": "click", "x": x, "y": y}));
    }

    pub fn right_click(&mut self, x: f64, y: f64) {
        self.op(&json!({"op": "right_click", "x": x, "y": y}));
    }

    pub fn move_pointer(&mut self, x: f64, y: f64) {
        self.op(&json!({"op": "move", "x": x, "y": y}));
    }

    pub fn key(&mut self, keyval: u32) {
        self.op(&json!({"op": "key", "keyval": keyval}));
    }

    pub fn key_with(&mut self, keyval: u32, state: u32) {
        self.op(&json!({"op": "key", "keyval": keyval, "state": state}));
    }

    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.key(u32::from(ch));
        }
    }

    pub fn double_click(&mut self, x: f64, y: f64, gtk_sequence: bool) {
        self.op(&json!({"op": "double_click", "x": x, "y": y, "gtk_sequence": gtk_sequence}));
    }

    pub fn fail_save(&mut self, kind: Option<&str>) {
        self.op(&json!({"op": "fail_save", "kind": kind}));
    }

    pub fn state(&mut self) -> Value {
        self.op(&json!({"op": "state"}))
    }

    pub fn snapshot_ink(&mut self) -> Value {
        self.op(&json!({"op": "snapshot"}))
    }

    pub fn phases(&mut self) -> Vec<(String, f64)> {
        self.state()["phases"]
            .as_array()
            .map(|phases| {
                phases
                    .iter()
                    .filter_map(|phase| Some((phase[0].as_str()?.to_owned(), phase[1].as_f64()?)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn snapshot_png(&mut self, path: &Path) -> Value {
        self.op(&json!({"op": "snapshot", "png": path}))
    }

    pub fn wait_page_started(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !self
            .phases()
            .iter()
            .any(|(name, _)| name == "first_text_called")
        {
            assert!(
                Instant::now() < deadline,
                "the page never asked for first text"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn wait_first_text(&mut self) -> f64 {
        self.wait_page_started();
        let script = "(async()=>{for(;;){const mark=performance.getEntriesByName('ss-first-text')[0];if(mark)return mark.detail;await new Promise((done)=>setTimeout(done,5))}})()";
        self.try_js(script, 60_000)
            .unwrap_or_else(|error| panic!("first text never appeared: {error}"))
            .as_f64()
            .expect("the first text mark holds a time")
    }

    pub fn memory(&self) -> Memory {
        memory_of_tree(self.pid)
    }

    fn stop(&mut self) {
        let tree = descendants(self.pid);
        for pid in tree.iter().rev() {
            unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
        }
        let _ = self.child.wait();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline
            && tree
                .iter()
                .any(|pid| Path::new(&format!("/proc/{pid}/cmdline")).exists())
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = std::fs::remove_file(&self.socket);
        if !self.keep_data {
            let _ = std::fs::remove_dir_all(&self.data);
        }
    }

    pub fn close(mut self) {
        self.stop();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

fn connect(socket: &Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return stream,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            Err(error) => panic!("the control socket never opened: {error}"),
        }
    }
}

#[derive(Clone)]
pub struct Listener {
    pub port: u16,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Listener {
    pub fn start() -> Listener {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("loopback listener");
        let port = listener.local_addr().expect("listener address").port();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let recorded = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let recorded = recorded.clone();
                std::thread::spawn(move || serve_probe(stream, &recorded));
            }
        });
        Listener { port, hits }
    }

    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn hits(&self) -> Vec<String> {
        self.hits.lock().expect("hit list").clone()
    }

    pub fn self_test(&self) -> bool {
        let path = "/K/listener_self_test";
        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", self.port)) else {
            return false;
        };
        let _ = write!(stream, "GET {path} HTTP/1.0\r\n\r\n");
        let mut reply = String::new();
        let _ = stream.read_to_string(&mut reply);
        reply.starts_with("HTTP/1.0 200") && self.hits().iter().any(|hit| hit == path)
    }
}

fn serve_probe(stream: TcpStream, hits: &Mutex<Vec<String>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return;
    }
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or("<no request line>");
    let path = path.split('?').next().unwrap_or(path).to_owned();
    hits.lock().expect("hit list").push(path);
    let _ = writer.write_all(
        b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\n\r\nok",
    );
}

pub fn data_files_besides_covers(data: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
                continue;
            }
            let name = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            let parts: Vec<&str> = name.split('/').collect();
            let cover = matches!(parts.as_slice(), ["books", key, "cover.jpg"]
                if key.len() == 32 && key.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
            if !cover {
                out.push(name);
            }
        }
    }
    let mut out = Vec::new();
    walk(data, data, &mut out);
    out.sort();
    out
}

pub fn place_name(data: &Path, book: &Path) -> String {
    let library = jadeslip_core::Library::open(jadeslip_core::LibraryConfig {
        data_dir: data.to_owned(),
    })
    .expect("the data folder opens");
    let key = library.key_of_file(book).expect("the book has a key");
    format!("books/{}/shelf.json", jadeslip_core::key_hex(key))
}
