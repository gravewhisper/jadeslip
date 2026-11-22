use serde::Serialize;
use jadeslip_core::{CancelToken, Reader};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const LONG_POLL: Duration = Duration::from_millis(250);
const MAX_BATCH: usize = 400;
const MAX_TITLE_CHARS: usize = 300;

#[derive(Serialize)]
pub struct HeadingBatch {
    titles: Vec<(u32, String)>,
    done: bool,
}

#[derive(Default)]
struct Inbox {
    titles: Vec<(u32, String)>,
    done: bool,
}

struct Run {
    id: u64,
    cancel: CancelToken,
    inbox: Mutex<Inbox>,
    arrived: Condvar,
}

impl Run {
    fn inbox(&self) -> MutexGuard<'_, Inbox> {
        self.inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Default)]
pub struct Headings {
    current: Mutex<Option<Arc<Run>>>,
}

impl Headings {
    fn current(&self) -> MutexGuard<'_, Option<Arc<Run>>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn start(&self, reader: Arc<Reader>, id: u64) {
        let run = Arc::new(Run {
            id,
            cancel: CancelToken::default(),
            inbox: Mutex::new(Inbox::default()),
            arrived: Condvar::new(),
        });
        if let Some(old) = self.current().replace(run.clone()) {
            old.cancel.cancel();
        }
        std::thread::spawn(move || {
            let cancel = run.cancel.clone();
            reader.heading_titles(&cancel, &mut |ix, title| {
                run.inbox()
                    .titles
                    .push((ix.0, title.chars().take(MAX_TITLE_CHARS).collect()));
                run.arrived.notify_all();
            });
            run.inbox().done = true;
            run.arrived.notify_all();
        });
    }

    pub fn cancel(&self, id: u64) {
        if let Some(run) = self.current().as_ref().filter(|run| run.id == id) {
            run.cancel.cancel();
        }
    }

    pub fn next(&self, id: u64) -> HeadingBatch {
        let run = self.current().as_ref().filter(|run| run.id == id).cloned();
        let Some(run) = run else {
            return HeadingBatch {
                titles: Vec::new(),
                done: true,
            };
        };
        let until = Instant::now() + LONG_POLL;
        let mut inbox = run.inbox();
        while inbox.titles.is_empty() && !inbox.done {
            let now = Instant::now();
            if now >= until {
                break;
            }
            inbox = run
                .arrived
                .wait_timeout(inbox, until - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        let count = inbox.titles.len().min(MAX_BATCH);
        let titles: Vec<_> = inbox.titles.drain(..count).collect();
        HeadingBatch {
            titles,
            done: inbox.done && inbox.titles.is_empty(),
        }
    }
}
