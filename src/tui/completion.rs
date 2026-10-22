//! What the command line suggests while a command is typed: the commands that start with (or
//! mention) what's typed, then the values of its first argument once the command is typed.

use crate::command::{ALIASES, COMMANDS, Command, CommandInfo};

/// A suggestion for the command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// The whole line once it's taken.
    pub line: String,
    /// The word it completes, as shown.
    pub word: String,
    /// The command's arguments (for commands), shown after the word.
    pub args: &'static str,
    /// What it does.
    pub about: String,
}

/// What to suggest for the line typed so far.
#[derive(Default, Debug)]
pub struct Completion {
    /// The command being given arguments, once its name is typed.
    pub command: Option<&'static CommandInfo>,
    pub suggestions: Vec<Suggestion>,
}

/// The command named `name`, or by one of its aliases.
pub fn find(name: &str) -> Option<&'static CommandInfo> {
    let name = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, command)| *command);
    COMMANDS.iter().find(|c| c.name == name)
}

/// The command `line` (the last of them) gives, if it's a whole one.
pub fn find_line(line: &str) -> Option<Command> {
    let last = line.rsplit(';').next()?.trim();
    crate::command::parse(last).ok()?.pop()
}

/// The suggestions for `line`: after the last `;`, the commands for the first word, or the values
/// of the first argument.
pub fn complete(line: &str) -> Completion {
    let start = line.rfind(';').map_or(0, |i| i + 1);
    let (head, segment) = line.split_at(start);
    let head = format!(
        "{head}{}",
        &segment[..segment.len() - segment.trim_start().len()]
    );
    let segment = segment.trim_start();

    let Some((name, rest)) = segment.split_once(char::is_whitespace) else {
        return Completion {
            command: None,
            suggestions: commands(&head, segment),
        };
    };
    let Some(command) = find(name) else {
        return Completion::default();
    };
    let rest = rest.trim_start();
    // Values for the first argument only.
    let suggestions = if rest.contains(char::is_whitespace) {
        Vec::new()
    } else {
        command
            .values
            .iter()
            .filter(|v| v.starts_with(rest) && **v != rest)
            .map(|v| Suggestion {
                line: format!("{head}{name} {v}"),
                word: (*v).into(),
                args: "",
                about: String::new(),
            })
            .collect()
    };
    Completion {
        command: Some(command),
        suggestions,
    }
}

/// The commands for a first word `word`: those starting with it (or one of their aliases), then
/// those it's part of or whose description mentions it (for three letters or more).
fn commands(head: &str, word: &str) -> Vec<Suggestion> {
    let suggestion = |c: &CommandInfo, alias: Option<&str>| Suggestion {
        line: format!(
            "{head}{}{}",
            c.name,
            if c.args.is_empty() { "" } else { " " }
        ),
        word: match alias {
            Some(alias) => format!("{alias} ({})", c.name),
            None => c.name.into(),
        },
        args: c.args,
        about: c.about.into(),
    };
    if word.is_empty() {
        return COMMANDS.iter().map(|c| suggestion(c, None)).collect();
    }
    let lower = word.to_lowercase();
    let mut starting: Vec<&CommandInfo> = COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(&lower))
        .collect();
    starting.sort_by_key(|c| (c.name.len(), c.name));
    let mut found: Vec<Suggestion> = starting.iter().map(|c| suggestion(c, None)).collect();
    for (alias, name) in ALIASES {
        if alias.starts_with(&lower)
            && let Some(c) = find(name)
            && !starting.iter().any(|s| s.name == c.name)
        {
            found.push(suggestion(c, Some(alias)));
        }
    }
    for c in COMMANDS {
        // Descriptions only for whole-ish words: "fo" isn't looking for "forward".
        let mentions =
            c.name.contains(&lower) || lower.len() > 2 && c.about.to_lowercase().contains(&lower);
        let new = suggestion(c, None);
        if mentions && !found.iter().any(|s| s.line == new.line) {
            found.push(new);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        complete(line)
            .suggestions
            .into_iter()
            .map(|s| s.word)
            .collect()
    }

    #[test]
    fn suggests_commands_starting_with_the_word_first() {
        let found = words("vol");
        assert_eq!(found[..2], ["volup", "voldown"].map(String::from)[..]);
        assert_eq!(words("pl")[0], "play");
        assert_eq!(words("x")[0], "x (quit)");
        assert_eq!(words("q")[0], "quit");
        // Then what mentions it.
        assert!(words("lyrics").contains(&"panel".to_string()));
        assert!(words("zzz").is_empty());
        assert_eq!(words("").len(), COMMANDS.len());
    }

    #[test]
    fn takes_a_command_with_a_space_if_it_has_arguments() {
        let found = complete("foc").suggestions;
        assert_eq!(found[0].line, "focus ");
        assert_eq!(complete("qui").suggestions[0].line, "quit");
    }

    #[test]
    fn suggests_values_after_the_command() {
        let completion = complete("focus l");
        assert_eq!(completion.command.unwrap().name, "focus");
        let lines: Vec<String> = completion.suggestions.into_iter().map(|s| s.line).collect();
        assert_eq!(lines, ["focus library", "focus lyrics"]);
        // Aliases, and the last of several commands.
        assert_eq!(words("loop t"), ["track"]);
        assert_eq!(
            complete("next; shuffle o").suggestions[0].line,
            "next; shuffle on"
        );
        // Nothing once the value is typed, or for unknown commands.
        assert!(words("shuffle on").is_empty());
        assert!(complete("nope x").command.is_none());
    }
}
