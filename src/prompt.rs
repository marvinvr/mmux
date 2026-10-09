//! Reading the question an agent is waiting on off its screen, and turning an answer
//! (`2`, `yes`, an option's text) into the keys that pick it.
//!
//! Agents ask in two shapes. The common one is a numbered menu with a cursor — Claude
//! Code's and Codex's permission prompts, Claude's multiple-choice questions, a
//! folder-trust dialog:
//!
//! ```text
//!  Do you want to proceed?
//!  ❯ 1. Yes
//!    2. Yes, and don't ask again for rm commands in /repo
//!    3. No, and tell Claude what to do differently (esc)
//! ```
//!
//! The other is a plain `[y/N]` line. A numbered list alone is not a menu — an agent
//! echoes a numbered prompt it was sent, and answers in numbered lists — so a menu also
//! needs a cursor on one of its options, numbering that runs from 1, a selection hint
//! (`Esc to cancel`, `Enter to select`, `(esc)`, …) and to sit at the bottom of the
//! screen, where a prompt waiting for an answer is drawn. Pure functions over screen
//! lines, so the heuristics are tested directly.

use serde::{Deserialize, Serialize};

/// Glyphs agents draw before the highlighted option.
const CURSORS: &[char] = &['❯', '›', '>', '▶', '►', '→', '▸', '➤'];
/// Phrases a selection menu prints and an agent's own output rarely does. Each is
/// matched lowercased; `esc to interrupt` (an agent at work) is deliberately absent.
const MENU_HINTS: &[&str] = &[
    "(esc)",
    "esc to cancel",
    "esc to exit",
    "esc to go back",
    "enter to select",
    "enter to confirm",
    "enter to continue",
    "press enter",
    "to navigate",
    "↑/↓",
];
/// How many screen lines from the bottom a prompt is looked for in.
const WINDOW: usize = 40;
/// Lines that may separate two options (an option's description, a divider).
const MAX_GAP: usize = 6;
/// A menu's last option must be among this many non-empty lines at the bottom.
const NEAR_BOTTOM: usize = 12;

/// A question on an agent's screen.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// The line that asks it (`Do you want to proceed?`), when one could be found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// A menu's options, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// A `[y/n]` line rather than a menu: answered by typing `y` or `n`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub yes_no: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One option of a menu.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub n: u32,
    pub label: String,
    /// The cursor is on it — what Enter alone would pick.
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
}

/// What answering takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Move the cursor `moves` options (negative: up), then press Enter.
    Select { moves: i32, choice: Choice },
    /// Type this text, then press Enter.
    Type(String),
}

/// The prompt at the bottom of `lines` (the visible screen, top to bottom), if any.
pub fn parse(lines: &[String]) -> Option<Prompt> {
    let lines: Vec<&str> = lines.iter().map(|l| unbox(l)).collect();
    let end = lines.iter().rposition(|l| !l.is_empty())?;
    let lines = &lines[end.saturating_sub(WINDOW - 1)..=end];
    menu(lines).or_else(|| yes_no(lines))
}

/// A row with any box border stripped (`│ text │`), trimmed.
fn unbox(line: &str) -> &str {
    let borders: &[char] = &['│', '┃', '║'];
    line.trim()
        .trim_start_matches(borders)
        .trim_end_matches(borders)
        .trim()
}

/// `❯ 2. Yes, and…` → `(true, 2, "Yes, and…")`.
fn option(line: &str) -> Option<(bool, u32, &str)> {
    let (selected, rest) = match line.strip_prefix(CURSORS) {
        Some(rest) => (true, rest.trim_start()),
        None => (false, line),
    };
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits == 0 || digits > 2 {
        return None;
    }
    let n: u32 = rest[..digits].parse().ok()?;
    let after = rest[digits..].strip_prefix(['.', ')'])?;
    if !after.starts_with(' ') {
        return None;
    }
    let label = after.trim();
    (!label.is_empty()).then_some((selected, n, label))
}

fn menu(lines: &[&str]) -> Option<Prompt> {
    let opts: Vec<Option<(bool, u32, &str)>> = lines.iter().map(|l| option(l)).collect();
    let cursor = opts
        .iter()
        .rposition(|o| o.is_some_and(|(sel, _, _)| sel))?;
    let (_, k, _) = opts[cursor]?;
    // Walk out from the cursor while the numbering stays consecutive.
    let mut found = vec![cursor];
    for (step, mut expect) in [(-1i64, k.checked_sub(1)), (1, k.checked_add(1))] {
        let mut i = cursor as i64;
        let mut gap = 0;
        while let Some(want) = expect.filter(|&n| n >= 1) {
            i += step;
            let Some(o) = usize::try_from(i).ok().and_then(|i| opts.get(i)) else {
                break;
            };
            match o {
                Some((_, n, _)) if *n == want => {
                    found.push(i as usize);
                    gap = 0;
                    expect = if step < 0 {
                        want.checked_sub(1)
                    } else {
                        want.checked_add(1)
                    };
                }
                Some(_) => break,
                None => {
                    gap += 1;
                    if gap > MAX_GAP {
                        break;
                    }
                }
            }
        }
    }
    found.sort_unstable();
    let (first, last) = (*found.first()?, *found.last()?);
    if found.len() < 2 || opts[first]?.1 != 1 {
        return None;
    }
    let below = lines[last + 1..].iter().filter(|l| !l.is_empty()).count();
    if below >= NEAR_BOTTOM {
        return None;
    }
    let tail = lines[first..].join("\n").to_lowercase();
    if !MENU_HINTS.iter().any(|h| tail.contains(h)) {
        return None;
    }
    let choices = found
        .iter()
        .filter_map(|&i| opts[i])
        .map(|(selected, n, label)| Choice {
            n,
            label: label.to_string(),
            selected,
        })
        .collect();
    Some(Prompt {
        question: question_above(&lines[..first]),
        choices,
        yes_no: false,
    })
}

/// The line asking the question: the nearest one above the options ending in `?`
/// (Codex puts the command between its question and the menu), else the nearest
/// non-empty one that isn't just a border.
fn question_above(lines: &[&str]) -> Option<String> {
    let text: Vec<&str> = lines
        .iter()
        .rev()
        .copied()
        .filter(|l| l.chars().any(char::is_alphanumeric))
        .take(8)
        .collect();
    text.iter()
        .find(|l| l.ends_with('?'))
        .or(text.first())
        .map(|l| l.to_string())
}

fn yes_no(lines: &[&str]) -> Option<Prompt> {
    let marks = ["(y/n)", "[y/n]", "(yes/no)", "[yes/no]", "y/n?"];
    lines
        .iter()
        .rev()
        .filter(|l| !l.is_empty())
        .take(2)
        .find(|l| {
            let l = l.to_lowercase();
            marks.iter().any(|m| l.contains(m))
        })
        .map(|l| Prompt {
            question: Some(l.to_string()),
            choices: Vec::new(),
            yes_no: true,
        })
}

/// How to give `answer` to `prompt`: a choice number, `yes`/`no` (`y`/`n`), or text
/// matching one option. A `[y/n]` line takes `y`/`n` (anything else is typed as is);
/// with no prompt on screen, the answer is typed as a reply. Errors name the choices.
pub fn plan(prompt: Option<&Prompt>, answer: &str) -> Result<Plan, String> {
    let answer = answer.trim();
    let word = answer.to_lowercase();
    let yes = matches!(word.as_str(), "yes" | "y");
    let no = matches!(word.as_str(), "no" | "n");
    let Some(prompt) = prompt.filter(|p| p.yes_no || !p.choices.is_empty()) else {
        return Ok(Plan::Type(answer.to_string()));
    };
    if prompt.yes_no {
        return Ok(Plan::Type(match (yes, no) {
            (true, _) => "y".into(),
            (_, true) => "n".into(),
            _ => answer.to_string(),
        }));
    }
    let choices = &prompt.choices;
    let pick = if let Ok(n) = word.parse::<u32>() {
        choices
            .iter()
            .position(|c| c.n == n)
            .ok_or_else(|| format!("there is no option {n}"))?
    } else if yes || no {
        let want = if yes { "yes" } else { "no" };
        choices
            .iter()
            .position(|c| starts_with_word(&c.label.to_lowercase(), want))
            .ok_or_else(|| format!("no option starts with “{want}”"))?
    } else {
        let hits: Vec<usize> = (0..choices.len())
            .filter(|&i| choices[i].label.to_lowercase().contains(&word))
            .collect();
        let leading: Vec<usize> = hits
            .iter()
            .copied()
            .filter(|&i| choices[i].label.to_lowercase().starts_with(&word))
            .collect();
        match (hits.as_slice(), leading.as_slice()) {
            ([i], _) | (_, [i]) if !word.is_empty() => *i,
            ([], _) => {
                return Err(format!(
                    "“{answer}” is none of the options — to type free text, use `mmux send`"
                ))
            }
            _ => return Err(format!("“{answer}” matches more than one option")),
        }
    };
    let at = choices.iter().position(|c| c.selected).unwrap_or(0);
    Ok(Plan::Select {
        moves: pick as i32 - at as i32,
        choice: choices[pick].clone(),
    })
}

/// `label` begins with the whole word `word` (`No, and…` for `no`; not `None`).
fn starts_with_word(label: &str, word: &str) -> bool {
    label
        .strip_prefix(word)
        .is_some_and(|rest| !rest.starts_with(char::is_alphanumeric))
}

/// The choices as one line, for an error or a report: `1. Yes · 2. No`.
pub fn list(choices: &[Choice]) -> String {
    choices
        .iter()
        .map(|c| format!("{}. {}", c.n, c.label))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    const CLAUDE: &str = "\
● I'll clean the build directory.

╭──────────────────────────────────────────────╮
│ Bash command                                 │
│                                              │
│   rm -rf build                               │
│   Remove the build output                    │
│                                              │
│ Do you want to proceed?                      │
│ ❯ 1. Yes                                     │
│   2. Yes, and don't ask again for rm in /x   │
│   3. No, and tell Claude what to do (esc)    │
╰──────────────────────────────────────────────╯
";

    const CODEX: &str = "\
  Would you like to run the following command?

  $ cargo test

› 1. Yes, proceed (y)
  2. Yes, and don't ask again for this command (a)
  3. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel
";

    const QUESTION: &str = "\
 ☐ Approach

 Which approach should we take?

   1. Rewrite it
      Start over with a cleaner design
 ❯ 2. Patch it
      Smallest change that fixes the bug
   3. Type something.
 ──────────────────────
   4. Chat about this

 Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn reads_claude_codex_and_question_menus() {
        let p = parse(&screen(CLAUDE)).unwrap();
        assert_eq!(p.question.as_deref(), Some("Do you want to proceed?"));
        assert_eq!(p.choices.len(), 3);
        assert!(p.choices[0].selected);
        assert_eq!(p.choices[1].label, "Yes, and don't ask again for rm in /x");

        let p = parse(&screen(CODEX)).unwrap();
        assert_eq!(
            p.question.as_deref(),
            Some("Would you like to run the following command?")
        );
        assert_eq!(p.choices.len(), 3);

        let p = parse(&screen(QUESTION)).unwrap();
        assert_eq!(
            p.question.as_deref(),
            Some("Which approach should we take?")
        );
        assert_eq!(
            p.choices.iter().map(|c| c.n).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert!(p.choices[1].selected);
    }

    #[test]
    fn a_numbered_list_is_not_a_menu() {
        // An echoed numbered prompt while the agent works: cursor-like glyph, but no
        // selection hint (`esc to interrupt` doesn't count).
        let echoed = "\
> 1. fix the tests
  2. update the docs

✻ Working… (esc to interrupt)
";
        assert_eq!(parse(&screen(echoed)), None);
        // An answer in a numbered list: no cursor at all.
        let answer = "\
Done. I changed:
1. the parser
2. the docs
Press Enter to continue
";
        assert_eq!(parse(&screen(answer)), None);
        // A menu that has scrolled far above the bottom is history, not a prompt.
        let old = format!("{CLAUDE}{}", "output line\n".repeat(NEAR_BOTTOM));
        assert_eq!(parse(&screen(&old)), None);
        // Numbering must start at 1.
        let partial = "❯ 2. Yes\n  3. No (esc)\n";
        assert_eq!(parse(&screen(partial)), None);
    }

    #[test]
    fn reads_a_yes_no_line() {
        let p = parse(&screen("Overwrite config.toml? [y/N] ")).unwrap();
        assert!(p.yes_no);
        assert_eq!(p.question.as_deref(), Some("Overwrite config.toml? [y/N]"));
        assert_eq!(parse(&screen("$ ls\nsrc  docs\n$ ")), None);
    }

    #[test]
    fn plans_numbers_yes_no_and_label_text() {
        let claude = parse(&screen(CLAUDE)).unwrap();
        let moves = |a: &str| match plan(Some(&claude), a) {
            Ok(Plan::Select { moves, .. }) => moves,
            other => panic!("{a}: {other:?}"),
        };
        assert_eq!(moves("1"), 0);
        assert_eq!(moves("3"), 2);
        assert_eq!(moves("yes"), 0);
        assert_eq!(moves("No"), 2);
        assert_eq!(moves("don't ask"), 1);
        assert!(plan(Some(&claude), "7").is_err());
        assert!(plan(Some(&claude), "maybe later").is_err());
        // Two options contain "yes": ambiguous unless only one starts with it.
        assert!(plan(Some(&claude), "and").is_err());

        // From a cursor further down, moving up is negative.
        let q = parse(&screen(QUESTION)).unwrap();
        assert!(matches!(
            plan(Some(&q), "1"),
            Ok(Plan::Select { moves: -1, .. })
        ));
        assert!(matches!(
            plan(Some(&q), "chat"),
            Ok(Plan::Select { moves: 2, .. })
        ));

        let yn = parse(&screen("Continue? (y/n)")).unwrap();
        assert_eq!(plan(Some(&yn), "yes"), Ok(Plan::Type("y".into())));
        assert_eq!(plan(Some(&yn), "N"), Ok(Plan::Type("n".into())));

        // Nothing on screen to pick from: the answer is typed as a reply.
        assert_eq!(plan(None, " use A "), Ok(Plan::Type("use A".into())));
    }
}
