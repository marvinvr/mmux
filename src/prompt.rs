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
//! Some menus aren't numbered — Claude Code's folder-trust dialog, shown before an
//! agent in a folder it hasn't seen does anything else:
//!
//! ```text
//!  ❯ No, exit
//!    Yes, I trust this folder
//!
//!  Enter to confirm · Esc to cancel
//! ```
//!
//! Those are read by column instead: the options are the lines aligned with the cursor
//! line's label, numbered 1… in order. The other shape is a plain `[y/N]` line. A
//! numbered list alone is not a menu — an agent echoes a numbered prompt it was sent,
//! and answers in numbered lists — so a menu also needs a cursor on one of its options,
//! numbering that runs from 1, a selection hint (`Esc to cancel`, `Enter to select`,
//! `(esc)`, …) and to sit at the bottom of the screen, where a prompt waiting for an
//! answer is drawn. Pure functions over screen lines, so the heuristics are tested
//! directly.

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
/// An unnumbered menu's selection hint must be among this many non-empty lines below
/// its last option: with no numbering to go by, a hint further off (or none) leaves a
/// multi-line message echoed after a `❯` looking just like one.
const BARE_HINT_WITHIN: usize = 3;

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
    let rows: Vec<(usize, &str)> = lines.iter().map(|l| unbox(l)).collect();
    let end = rows.iter().rposition(|(_, l)| !l.is_empty())?;
    let rows = &rows[end.saturating_sub(WINDOW - 1)..=end];
    let lines: Vec<&str> = rows.iter().map(|&(_, l)| l).collect();
    menu(&lines)
        .or_else(|| bare_menu(rows))
        .or_else(|| yes_no(&lines))
}

/// A row with any box border stripped (`│ text │`), trimmed, and the column its text
/// starts at — what lines an unnumbered menu's options up.
fn unbox(line: &str) -> (usize, &str) {
    let borders: &[char] = &['│', '┃', '║'];
    let text = line
        .trim_start()
        .trim_start_matches(borders)
        .trim_start()
        .trim_end()
        .trim_end_matches(borders)
        .trim_end();
    let indent = line[..line.len()
        - line
            .trim_start_matches(|c: char| c.is_whitespace() || borders.contains(&c))
            .len()]
        .chars()
        .count();
    (indent, text)
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

/// A menu without numbers (see the module docs): a cursor line, the lines lined up
/// with its label above and below it (a deeper-indented description between them is
/// skipped), and a selection hint right under the last one.
fn bare_menu(rows: &[(usize, &str)]) -> Option<Prompt> {
    // An option's label: text that starts with a letter or digit and isn't the hint.
    let label = |text: &str| {
        let lower = text.to_lowercase();
        text.starts_with(char::is_alphanumeric) && !MENU_HINTS.iter().any(|h| lower.contains(h))
    };
    let (cursor, col) = rows
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, &(indent, text))| {
            let rest = text.strip_prefix(CURSORS)?;
            let name = rest.trim_start();
            (rest.starts_with(' ') && label(name) && option(text).is_none())
                .then(|| (i, indent + text.chars().count() - name.chars().count()))
        })?;
    let mut found = vec![cursor];
    for step in [-1i64, 1] {
        let mut i = cursor as i64 + step;
        while let Some(&(indent, text)) = usize::try_from(i).ok().and_then(|i| rows.get(i)) {
            match () {
                _ if text.is_empty() || indent < col => break,
                _ if indent > col => {}
                _ if label(text) => found.push(i as usize),
                _ => break,
            }
            i += step;
        }
    }
    found.sort_unstable();
    let (first, last) = (*found.first()?, *found.last()?);
    if found.len() < 2 {
        return None;
    }
    let below: Vec<&str> = rows[last + 1..]
        .iter()
        .map(|&(_, l)| l)
        .filter(|l| !l.is_empty())
        .collect();
    if below.len() >= NEAR_BOTTOM {
        return None;
    }
    let hint = below.iter().take(BARE_HINT_WITHIN).any(|l| {
        let l = l.to_lowercase();
        MENU_HINTS.iter().any(|h| l.contains(h))
    });
    if !hint {
        return None;
    }
    let choices = found
        .iter()
        .zip(1..)
        .map(|(&i, n)| {
            let text = rows[i].1;
            Choice {
                n,
                label: text.trim_start_matches(CURSORS).trim().to_string(),
                selected: i == cursor,
            }
        })
        .collect();
    let above: Vec<&str> = rows[..first].iter().map(|&(_, l)| l).collect();
    Some(Prompt {
        question: question_above(&above),
        choices,
        yes_no: false,
    })
}

/// The line asking the question: the nearest one above the options ending in `?`
/// (Codex puts the command between its question and the menu), else the nearest one
/// with a question in it, up to its `?` (a question wrapped into the paragraph after
/// it, as in Claude's trust dialog), else the nearest non-empty one that isn't just a
/// border.
fn question_above(lines: &[&str]) -> Option<String> {
    let text: Vec<&str> = lines
        .iter()
        .rev()
        .copied()
        .filter(|l| l.chars().any(char::is_alphanumeric))
        .take(8)
        .collect();
    let asked = || text.iter().find_map(|l| l.find("? ").map(|at| &l[..=at]));
    text.iter()
        .find(|l| l.ends_with('?'))
        .copied()
        .or_else(asked)
        .or(text.first().copied())
        .map(str::to_string)
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
/// with no prompt on screen, a reply is typed — but not an answer that only makes
/// sense as a pick (see [`looks_like_a_pick`]): typed into a menu mmux couldn't read,
/// its Enter would choose whatever the cursor is on. Errors name the choices.
pub fn plan(prompt: Option<&Prompt>, answer: &str) -> Result<Plan, String> {
    let answer = answer.trim();
    let word = answer.to_lowercase();
    let yes = matches!(word.as_str(), "yes" | "y");
    let no = matches!(word.as_str(), "no" | "n");
    let Some(prompt) = prompt.filter(|p| p.yes_no || !p.choices.is_empty()) else {
        if looks_like_a_pick(answer) {
            return Err(format!(
                "no prompt detected on screen, and “{answer}” reads like a choice — typed \
blindly into a menu mmux can't read, its Enter would pick whatever the cursor is on. \
Check the screen, then pick with `mmux keys` (Up/Down, Enter), or pass --text to type it anyway"
            ));
        }
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

/// Whether `answer` reads as picking an option rather than replying: a bare number, or
/// one that opens with a menu word (`yes`, `No, exit`, `Allow`, `Yes, I trust this
/// folder`).
fn looks_like_a_pick(answer: &str) -> bool {
    const PICKS: &[&str] = &[
        "yes", "no", "y", "n", "ok", "okay", "allow", "deny", "accept", "decline", "reject",
        "approve", "cancel", "proceed", "continue", "always", "never", "skip", "exit", "quit",
        "trust", "don't", "dont",
    ];
    let lower = answer.trim().to_lowercase();
    lower.len() <= 2 && !lower.is_empty() && lower.chars().all(|c| c.is_ascii_digit())
        || PICKS.iter().any(|w| starts_with_word(&lower, w))
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

    /// Claude Code's folder-trust dialog, as a `mmux new agent --prompt …` in a folder
    /// it had never seen showed it: options without numbers, the cursor on "No".
    const TRUST: &str = "\
────────────────────────────────────────────────────────────────────────────────
 Accessing workspace:

 /Users/mvr/Development/Private/mealmate

 Quick safety check: Is this a project you created or one you trust? (Like your own code, a
 well-known open source project, or work from your team). If not, take a moment to review what's in
 this folder first.

 Claude Code'll be able to read, edit, and execute files here.

 Security guide

 ❯ No, exit
   Yes, I trust this folder

 Enter to confirm · Esc to cancel









";

    #[test]
    fn reads_claude_s_unnumbered_trust_dialog() {
        let mut rows = screen(TRUST);
        let p = parse(&rows).unwrap();
        assert_eq!(
            p.question.as_deref(),
            Some("Quick safety check: Is this a project you created or one you trust?")
        );
        assert_eq!(
            list(&p.choices),
            "1. No, exit · 2. Yes, I trust this folder"
        );
        assert!(p.choices[0].selected && !p.choices[1].selected);
        // The same dialog with its blank lines squeezed out, as pasted from a screen.
        rows.retain(|l| !l.trim().is_empty());
        assert_eq!(parse(&rows).unwrap().choices, p.choices);

        for answer in ["Yes, I trust this folder", "2", "yes", "trust"] {
            assert!(
                matches!(plan(Some(&p), answer), Ok(Plan::Select { moves: 1, .. })),
                "{answer}"
            );
        }
        assert!(matches!(
            plan(Some(&p), "No"),
            Ok(Plan::Select { moves: 0, .. })
        ));
        // Once trusted, the cursor sits on "Yes".
        let moved = TRUST
            .replace(" ❯ No, exit", "   No, exit")
            .replace("   Yes, I trust", " ❯ Yes, I trust");
        let p = parse(&screen(&moved)).unwrap();
        assert!(p.choices[1].selected);
        assert!(matches!(
            plan(Some(&p), "1"),
            Ok(Plan::Select { moves: -1, .. })
        ));
    }

    #[test]
    fn an_echoed_message_is_not_an_unnumbered_menu() {
        // A multi-line message shown after Claude's `❯`, with its idle footer below.
        let echoed = "\
❯ fix the parser
  then update the docs
⏺ Done.

────────────────────
❯
────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle)
";
        assert_eq!(parse(&screen(echoed)), None);
        // …and the same with a hint too far below to belong to it.
        let far = "\
❯ fix the parser
  then update the docs
⏺ Done.
one
two
Press enter to continue
";
        assert_eq!(parse(&screen(far)), None);
        // A lone cursor line is no menu either.
        assert_eq!(parse(&screen("❯ Yes\n\nEnter to confirm\n")), None);
    }

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

        // Nothing on screen to pick from: the answer is typed as a reply — unless it
        // only makes sense as a pick, which typed into an unread menu picks blindly.
        assert_eq!(plan(None, " use A "), Ok(Plan::Type("use A".into())));
        assert_eq!(
            plan(None, "nothing yet"),
            Ok(Plan::Type("nothing yet".into()))
        );
        for pick in [
            "2",
            "yes",
            "N",
            "Yes, I trust this folder",
            "No, exit",
            "allow",
        ] {
            assert!(plan(None, pick).is_err(), "{pick}");
        }
    }
}
