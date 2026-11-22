use fancy_regex::{Expander, Regex, RegexBuilder};
use jadeslip_core::{Case, PatternEngine, run_worker};

const NO_STEP_LIMIT: usize = usize::MAX;

struct FancyEngine;

impl PatternEngine for FancyEngine {
    type Compiled = Regex;

    fn compile(pattern: &str, case: Case) -> Result<Regex, String> {
        RegexBuilder::new(pattern)
            .case_insensitive(case == Case::Insensitive)
            .backtrack_limit(NO_STEP_LIMIT)
            .build()
            .map_err(|error| error.to_string())
    }

    fn check_template(compiled: &Regex, template: &str) -> Result<(), String> {
        Expander::default()
            .check(template, compiled)
            .map_err(|_| {
                "The replacement names a group this pattern does not have. Write ${1} to join a group to a letter."
                    .to_owned()
            })
    }

    fn matches(
        compiled: &Regex,
        text: &str,
        template: Option<&str>,
        emit: &mut dyn FnMut(usize, usize, Option<String>) -> bool,
    ) -> Result<(), String> {
        let Some(template) = template else {
            for found in compiled.find_iter(text) {
                let found = found.map_err(|error| error.to_string())?;
                if !emit(found.start(), found.end(), None) {
                    break;
                }
            }
            return Ok(());
        };
        let expander = Expander::default();
        for captures in compiled.captures_iter(text) {
            let captures = captures.map_err(|error| error.to_string())?;
            let whole = captures.get(0).ok_or("a match has no whole group")?;
            let replaced = expander.expansion(template, &captures);
            if !emit(whole.start(), whole.end(), Some(replaced)) {
                break;
            }
        }
        Ok(())
    }
}

fn main() {
    run_worker::<FancyEngine>()
}
