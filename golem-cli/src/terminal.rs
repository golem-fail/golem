//! What an interactive command says when it has no terminal: an agent that
//! runs golem cannot answer a prompt, so the message names a way on.

use std::io::IsTerminal;

use anyhow::{bail, Result};

/// Whether a prompt can run: dialoguer draws on stderr and reads the keys
/// from stdin, so a terminal on only one of them is not enough.
pub fn is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Fail without a terminal, naming each `(command, what it does)` that
/// works without one, and the command for the user to run in a terminal.
pub fn require(command: &str, without: &[(&str, &str)]) -> Result<()> {
    require_with(command, without, "")
}

/// [`require`], with a `note` line after the commands, such as the values
/// they take.
pub fn require_with(command: &str, without: &[(&str, &str)], note: &str) -> Result<()> {
    if is_interactive() {
        return Ok(());
    }
    bail!("{}", message(command, without, note))
}

fn message(command: &str, without: &[(&str, &str)], note: &str) -> String {
    let mut out = format!("{command} is interactive and needs a terminal. Without one:\n");
    let width = without.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    for (cmd, what) in without {
        out.push_str(&format!("  {cmd:<width$}    # {what}\n"));
    }
    if !note.is_empty() {
        out.push_str(&format!("  {note}\n"));
    }
    out.push_str(&format!(
        "  or ask the user to run `{command}` in a terminal"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_names_each_way_on() {
        assert_eq!(
            message(
                "golem install-script",
                &[(
                    "golem install-script --print FRAMEWORK",
                    "read a template, then adapt it"
                )],
                "frameworks: expo, flutter"
            ),
            "golem install-script is interactive and needs a terminal. Without one:\n  \
             golem install-script --print FRAMEWORK    # read a template, then adapt it\n  \
             frameworks: expo, flutter\n  \
             or ask the user to run `golem install-script` in a terminal"
        );
    }
}
