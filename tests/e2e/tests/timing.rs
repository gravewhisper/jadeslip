use serde_json::Value;
use jadeslip_e2e::{App, Memory, book_label, generated_book, largest_real_book};
use std::path::{Path, PathBuf};
use std::time::Duration;

const RUNS: usize = 9;
const FIRST_TURN: u64 = 40;
const TURNS: u64 = 200;

const INSTALL_TURNS: &str = r"(async()=>{
  const main = await import('/main.js');
  const frames = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  window.__turn = async (ix) => {
    const started = performance.now();
    await main.goTo(ix);
    const mounted = performance.now();
    await frames();
    return { mounted: mounted - started, painted: performance.now() - started };
  };
  return true;
})()";

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn percentile(sorted: &[f64], share: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * share).round() as usize]
}

struct Launch {
    first_text_ms: f64,
    phases: Vec<(String, f64)>,
    memory: Memory,
    ink: u64,
    text_len: u64,
}

fn one_launch(book: &Path) -> Launch {
    let mut app = App::launch(Some(book), &[]);
    let shown = app.wait_first_text();
    let memory = app.memory();
    let first_text_ms = shown - app.spawned_at_ms;
    let spawned = app.spawned_at_ms;
    let phases = app
        .phases()
        .into_iter()
        .map(|(name, at)| (name, at - spawned))
        .collect();
    let ink = app.snapshot_ink()["ink"].as_u64().unwrap_or(0);
    let text_len = app
        .js("document.getElementById('book-host').shadowRoot.textContent.length")
        .as_u64()
        .unwrap_or(0);
    app.assert_on_headless_output();
    app.close();
    Launch {
        first_text_ms,
        phases,
        memory,
        ink,
        text_len,
    }
}

fn report_launches(label: &str, launches: &[Launch]) {
    let mut first: Vec<f64> = launches.iter().map(|launch| launch.first_text_ms).collect();
    first.sort_by(f64::total_cmp);
    let mut rss: Vec<f64> = launches
        .iter()
        .map(|launch| launch.memory.rss_kb as f64 / 1024.0)
        .collect();
    let mut pss: Vec<f64> = launches
        .iter()
        .map(|launch| launch.memory.pss_kb as f64 / 1024.0)
        .collect();
    println!(
        "{label}: first text median {:.0} ms, range {:.0} to {:.0}, {} runs. RSS {:.0} MB, PSS {:.0} MB, processes {}, text units {}, smallest snapshot ink {}",
        first[first.len() / 2],
        first[0],
        first[first.len() - 1],
        first.len(),
        median(&mut rss),
        median(&mut pss),
        launches[0].memory.processes,
        launches[0].text_len,
        launches.iter().map(|launch| launch.ink).min().unwrap_or(0),
    );
    let names: Vec<String> = launches[0]
        .phases
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    for name in names {
        let mut at: Vec<f64> = launches
            .iter()
            .filter_map(|launch| {
                launch
                    .phases
                    .iter()
                    .find(|(phase, _)| *phase == name)
                    .map(|(_, at)| *at)
            })
            .collect();
        println!("  {name:<20} {:>7.1} ms", median(&mut at));
    }
}

fn check_launches(launches: &[Launch]) {
    for launch in launches {
        assert!(launch.text_len > 0, "the chapter had text");
        assert!(launch.ink > 200, "the snapshot showed text pixels");
    }
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh timing, headless output"]
fn time_to_first_text_and_memory() {
    let books: Vec<(String, PathBuf)> = vec![
        (book_label(&largest_real_book()), largest_real_book()),
        (
            "large-h1-div-br".to_owned(),
            generated_book("large-h1-div-br"),
        ),
        ("large-h1-p".to_owned(), generated_book("large-h1-p")),
    ];
    println!("HEADLESS OUTPUT, session lock state not controlled, page cache warm");
    let mut results: Vec<Vec<Launch>> = books.iter().map(|_| Vec::new()).collect();
    for book in &books {
        one_launch(&book.1);
    }
    for _ in 0..RUNS {
        for (slot, book) in results.iter_mut().zip(&books) {
            slot.push(one_launch(&book.1));
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    for (slot, book) in results.iter().zip(&books) {
        report_launches(&book.0, slot);
        check_launches(slot);
    }
}

fn turn_series(app: &mut App, indexes: &[u64]) -> (Vec<f64>, Vec<f64>) {
    let mut mounted = Vec::with_capacity(indexes.len());
    let mut painted = Vec::with_capacity(indexes.len());
    for ix in indexes {
        let result: Value = app.js(&format!("window.__turn({ix})"));
        mounted.push(result["mounted"].as_f64().expect("turn time"));
        painted.push(result["painted"].as_f64().expect("turn time"));
    }
    mounted.sort_by(f64::total_cmp);
    painted.sort_by(f64::total_cmp);
    (mounted, painted)
}

fn print_series(label: &str, (mounted, painted): &(Vec<f64>, Vec<f64>)) {
    println!(
        "{label}: {} turns, mounted p50 {:.1} p95 {:.1} max {:.1} ms, second rAF p50 {:.1} p95 {:.1} max {:.1} ms",
        mounted.len(),
        percentile(mounted, 0.5),
        percentile(mounted, 0.95),
        mounted[mounted.len() - 1],
        percentile(painted, 0.5),
        percentile(painted, 0.95),
        painted[painted.len() - 1],
    );
}

fn turns_on(label: &str, book: &Path, largest_item: Option<u64>) {
    let mut app = App::launch(Some(book), &[]);
    app.wait_first_text();
    app.assert_on_headless_output();
    let before = app.memory();
    app.js(INSTALL_TURNS);
    let indexes: Vec<u64> = (FIRST_TURN..FIRST_TURN + TURNS).collect();
    let warm = turn_series(&mut app, &indexes);
    print_series(label, &warm);
    let after = app.memory();
    println!(
        "  memory at first text RSS {} MB PSS {} MB, after {TURNS} turns RSS {} MB PSS {} MB",
        before.rss_kb / 1024,
        before.pss_kb / 1024,
        after.rss_kb / 1024,
        after.pss_kb / 1024
    );
    if let Some(item) = largest_item {
        let series = turn_series(&mut app, &[item, 41, item, 42, item, 43, item, 44, item]);
        print_series("  largest spine item and back", &series);
    }
    app.close();
}

fn largest_item_of(book: &Path) -> u64 {
    let reader = jadeslip_core::Reader::open(
        std::fs::File::open(book).expect("book opens"),
        jadeslip_core::DEFAULT_LIMITS,
    )
    .expect("book parses");
    reader
        .rows()
        .iter()
        .max_by_key(|row| row.rev.map_or(0, |rev| rev.inflated_len))
        .map_or(0, |row| u64::from(row.ix.0))
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh timing, headless output"]
fn chapter_turns_and_memory() {
    println!("HEADLESS OUTPUT, page cache warm");
    let largest = largest_real_book();
    turns_on(
        &book_label(&largest),
        &largest,
        Some(largest_item_of(&largest)),
    );
    turns_on("large-h1-div-br", &generated_book("large-h1-div-br"), None);
}
