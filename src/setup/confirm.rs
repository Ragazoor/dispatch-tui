//! The `Confirmer` seam: interactive yes/no prompts and a test fake.

use super::*;
use anyhow::bail;

/// Seam over interactive prompts so the setup/uninstall orchestration flows
/// (and, via [`prompt_text`](Confirmer::prompt_text), the startup host-label
/// gate — see `docs/specs/startup.allium`: `HostLabelPrompt`) can be driven
/// deterministically in tests. The real implementation ([`StdinConfirmer`])
/// reads from stdin; tests inject a fake that returns queued answers.
pub trait Confirmer {
    /// Prompt defaulting to **Yes** (empty input counts as yes).
    fn confirm(&self, prompt: &str) -> Result<bool>;

    /// Prompt defaulting to **No** — the user must explicitly type "y".
    fn confirm_dangerous(&self, prompt: &str) -> Result<bool>;

    /// Prompt for free text with a pre-filled `default`. Empty input (just
    /// pressing enter) accepts the default rather than being treated as a
    /// blank answer — this is what makes `startup.allium`'s
    /// `HostLabelPrompt` a one-keypress accept when the hostname is fine as
    /// the label.
    fn prompt_text(&self, prompt: &str, default: &str) -> Result<String>;
}

/// Real confirmer backed by stderr prompts and stdin input.
pub struct StdinConfirmer;

impl StdinConfirmer {
    /// Prompt on stderr and read a yes/no answer from stdin. `default_yes`
    /// selects both the displayed hint (`[Y/n]` vs `[y/N]`) and the meaning of
    /// empty input.
    fn prompt(&self, prompt: &str, default_yes: bool) -> Result<bool> {
        let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
        eprint!("{prompt} {hint} ");
        std::io::stderr().flush()?;
        read_yes_no(&mut std::io::stdin().lock(), default_yes)
    }
}

/// Read one line and interpret it as yes/no. End of input is an error, never
/// the default: a closed stdin or Ctrl-D is not an answer, so it must not
/// consent to a rewrite.
fn read_yes_no(reader: &mut impl std::io::BufRead, default_yes: bool) -> Result<bool> {
    let mut input = String::new();
    if reader.read_line(&mut input)? == 0 {
        bail!("no answer: input closed before the prompt was answered");
    }
    let trimmed = input.trim().to_lowercase();
    Ok(match trimmed.as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    })
}

/// Read one line of free text; empty input accepts `default`. End of input is
/// an error for the same reason as in [`read_yes_no`].
fn read_text(reader: &mut impl std::io::BufRead, default: &str) -> Result<String> {
    let mut input = String::new();
    if reader.read_line(&mut input)? == 0 {
        bail!("no answer: input closed before the prompt was answered");
    }
    let trimmed = input.trim();
    Ok(if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    })
}

impl Confirmer for StdinConfirmer {
    fn confirm(&self, prompt: &str) -> Result<bool> {
        self.prompt(prompt, true)
    }

    fn confirm_dangerous(&self, prompt: &str) -> Result<bool> {
        self.prompt(prompt, false)
    }

    fn prompt_text(&self, prompt: &str, default: &str) -> Result<String> {
        eprint!("{prompt} [{default}] ");
        std::io::stderr().flush()?;
        read_text(&mut std::io::stdin().lock(), default)
    }
}

/// A [`Confirmer`] that returns queued answers instead of reading stdin,
/// mirroring `MockProcessRunner`. Separate queues for the default-yes and
/// default-no (dangerous) prompts so tests assert which kind fired. Panics if a
/// prompt is issued with no queued answer — the same fail-loud contract as
/// `MockProcessRunner`.
///
/// Lives outside `mod tests` because two test modules drive prompts: this one
/// (uninstall) and `crate::startup`'s (the startup consent prompt). A second
/// copy could answer differently from this one and neither would look wrong.
#[cfg(test)]
pub(crate) struct FakeConfirmer {
    confirm_answers: std::sync::Mutex<std::collections::VecDeque<bool>>,
    dangerous_answers: std::sync::Mutex<std::collections::VecDeque<bool>>,
    text_answers: std::sync::Mutex<std::collections::VecDeque<String>>,
    confirm_calls: std::sync::Mutex<usize>,
    dangerous_calls: std::sync::Mutex<usize>,
    text_calls: std::sync::Mutex<usize>,
}

#[cfg(test)]
impl FakeConfirmer {
    pub(crate) fn new(confirm: Vec<bool>, dangerous: Vec<bool>) -> Self {
        Self::with_text(confirm, dangerous, vec![])
    }

    /// Like [`Self::new`], with a queue of text answers for
    /// [`Confirmer::prompt_text`] — `startup.allium`'s `HostLabelPrompt`.
    pub(crate) fn with_text(confirm: Vec<bool>, dangerous: Vec<bool>, text: Vec<String>) -> Self {
        Self {
            confirm_answers: std::sync::Mutex::new(confirm.into()),
            dangerous_answers: std::sync::Mutex::new(dangerous.into()),
            text_answers: std::sync::Mutex::new(text.into()),
            confirm_calls: std::sync::Mutex::new(0),
            dangerous_calls: std::sync::Mutex::new(0),
            text_calls: std::sync::Mutex::new(0),
        }
    }

    /// Confirmer that must never be prompted.
    pub(crate) fn never() -> Self {
        Self::new(vec![], vec![])
    }

    pub(crate) fn confirm_call_count(&self) -> usize {
        *self.confirm_calls.lock().unwrap()
    }

    pub(crate) fn dangerous_call_count(&self) -> usize {
        *self.dangerous_calls.lock().unwrap()
    }

    pub(crate) fn text_call_count(&self) -> usize {
        *self.text_calls.lock().unwrap()
    }
}

#[cfg(test)]
impl Confirmer for FakeConfirmer {
    fn confirm(&self, _prompt: &str) -> Result<bool> {
        *self.confirm_calls.lock().unwrap() += 1;
        Ok(self
            .confirm_answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeConfirmer: no confirm answer queued"))
    }

    fn confirm_dangerous(&self, _prompt: &str) -> Result<bool> {
        *self.dangerous_calls.lock().unwrap() += 1;
        Ok(self
            .dangerous_answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeConfirmer: no dangerous answer queued"))
    }

    fn prompt_text(&self, _prompt: &str, _default: &str) -> Result<String> {
        *self.text_calls.lock().unwrap() += 1;
        Ok(self
            .text_answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeConfirmer: no text answer queued"))
    }
}

#[cfg(test)]
mod eof_tests {
    use super::*;

    #[test]
    fn eof_is_not_consent() {
        assert!(read_yes_no(&mut "".as_bytes(), true).is_err());
        assert!(read_yes_no(&mut "".as_bytes(), false).is_err());
        assert!(read_text(&mut "".as_bytes(), "host").is_err());
    }

    #[test]
    fn empty_line_takes_the_default() {
        assert!(read_yes_no(&mut "\n".as_bytes(), true).unwrap());
        assert!(!read_yes_no(&mut "\n".as_bytes(), false).unwrap());
        assert_eq!(read_text(&mut "\n".as_bytes(), "host").unwrap(), "host");
    }

    #[test]
    fn explicit_answers() {
        assert!(read_yes_no(&mut "Yes\n".as_bytes(), false).unwrap());
        assert!(!read_yes_no(&mut "n\n".as_bytes(), true).unwrap());
        assert_eq!(read_text(&mut "box\n".as_bytes(), "host").unwrap(), "box");
    }
}
