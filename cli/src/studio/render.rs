//! Drawing the console. Hand-rolled ANSI, the same call
//! `commands/view.rs` made for markdown highlighting — and here it also
//! means every byte that reaches the terminal passes through code in this
//! file, which is what `sanitize` relies on.

use super::state::{Approval, Console};

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const REVERSE: &str = "\x1b[7m";
const RESET: &str = "\x1b[0m";

/// Strip everything a terminal would treat as a command rather than as
/// text.
///
/// The sandbox protects the host from the agent's *execution*. It does
/// nothing about the agent's *bytes*: a reply drawn straight into the
/// operator's terminal can move the cursor, rewrite lines that have
/// already been read, set the window title, or drive an OSC handler — so
/// an agent could make the console display something other than what it
/// actually said, which is the one thing an operator is relying on it
/// for. Every byte the console draws from a turn goes through here first.
///
/// Newline and tab survive because they are layout, not addressing.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\t' => out.push(c),
            // ESC: drop the whole sequence, not just the escape byte —
            // leaving the payload behind would print `[31m` at best and
            // reassemble into a live sequence at worst.
            '\x1b' => match chars.peek() {
                // CSI: parameters/intermediates, then one final byte.
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC and the other string-argument introducers run until
                // BEL or ST (ESC \).
                Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Two-character escapes (ESC c is a full terminal reset).
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            // Every other C0 control, plus DEL. \r is deliberately in
            // here: a lone carriage return rewrites the line just drawn.
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {}
            c => out.push(c),
        }
    }
    out
}

/// The live "still working" label: a frame of motion plus the elapsed
/// time. Both matter. Motion says the console is alive; elapsed time says
/// how long the agent has been at it, which is the only honest answer
/// available while turns are request/response and nothing streams back
/// (see docs/MCP.md, "What this does not do").
pub fn waiting_label(waited: std::time::Duration) -> String {
    const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
    let secs = waited.as_secs();
    let frame = FRAMES[(waited.as_millis() / 250) as usize % FRAMES.len()];
    if secs < 60 {
        format!("{frame} working {secs}s")
    } else {
        format!("{frame} working {}m{:02}s", secs / 60, secs % 60)
    }
}

/// Wrap to the terminal width after sanitizing — a single very long line
/// would otherwise push the rest of the frame off screen.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    // Floored at 1 only because chunking by zero would panic — the width
    // the caller asks for is the width it gets.
    let width = width.max(1);
    let mut lines = vec![];
    for line in sanitize(text).lines() {
        if line.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in line.split_whitespace() {
            if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
                lines.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            // A single word longer than the width still has to be broken.
            if word.chars().count() > width {
                for chunk in word.chars().collect::<Vec<_>>().chunks(width) {
                    lines.push(chunk.iter().collect());
                }
            } else {
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

/// The whole frame, as one string. Pure: takes state, returns bytes, so
/// what the console would draw can be asserted on directly.
pub fn frame(c: &Console, width: usize, height: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!("{BOLD}{CYAN}moor studio{RESET}{DIM}  ↑/↓ project · type to ask · Enter send · e edit · a approve · r refresh · q quit{RESET}\n\n"));

    for (i, p) in c.projects.iter().enumerate() {
        let marker = if i == c.selected { REVERSE } else { "" };
        let state = if let Some(waited) = p.waited() {
            format!("{YELLOW}{}{RESET}", waiting_label(waited))
        } else if p.up {
            format!("{GREEN}up{RESET}")
        } else {
            format!("{DIM}down{RESET}")
        };
        // Never inferred — whatever `keel next --json` said, or nothing.
        let stage = match (&p.slug, &p.stage) {
            (Some(slug), Some(stage)) => format!("{slug} · {stage}"),
            _ => format!("{DIM}no active spec{RESET}"),
        };
        out.push_str(&format!(
            "{marker} {:<24}{RESET} {state}  {stage}\n",
            p.name
        ));
    }
    out.push('\n');

    if let Approval::Armed {
        project,
        slug,
        stage,
        ..
    } = &c.approval
    {
        out.push_str(&format!(
            "{RED}{BOLD}approve {stage} stage of '{slug}' in project '{project}'? press y to confirm, any other key cancels{RESET}\n\n"
        ));
    }

    if let Some(p) = c.selected_project() {
        let body_height = height.saturating_sub(c.projects.len() + 8).max(3);
        let mut body: Vec<String> = vec![];
        for entry in &p.transcript {
            body.extend(wrap(&entry.render(), width));
        }
        let start = body.len().saturating_sub(body_height);
        for line in &body[start..] {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        // The same label again, immediately above the input line. The
        // project list at the top of the frame is eight lines away from
        // where the operator is actually looking, which is why a turn in
        // flight read as "no feedback at all".
        if let Some(waited) = p.waited() {
            out.push_str(&format!(
                "{YELLOW}{} — this turn is still running; keys still work{RESET}\n",
                waiting_label(waited)
            ));
        }
        out.push_str(&format!(
            "{BOLD}[{}]{RESET} {}▏\n",
            p.name,
            sanitize(&p.input)
        ));
    }

    if !c.status.is_empty() {
        out.push_str(&format!("{DIM}{}{RESET}\n", sanitize(&c.status)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-3
    #[test]
    fn render_strips_control_sequences() {
        // A reply that tries to address the terminal instead of talking to
        // the operator.
        let hostile = concat!(
            "gate passed\x1b[2J\x1b[H",        // clear screen, home cursor
            "\x1b[31mred\x1b[0m ",             // colour
            "\x1b]0;you have been hacked\x07", // OSC window title
            "\x1b]8;;http://evil.example\x1b\\link\x1b]8;;\x1b\\", // OSC 8 hyperlink
            "\rGATE FAILED",                   // rewrite the line just drawn
            "\x1bc",                           // full terminal reset
            "\x07\x00\x08 tail\ttabbed\nnewline",
        );
        let clean = sanitize(hostile);

        // No escape, no CSI/OSC payload, no bare C0 left behind.
        assert!(!clean.contains('\x1b'), "escape survived: {clean:?}");
        assert!(!clean.contains('\r'), "carriage return survived: {clean:?}");
        assert!(!clean.contains('\x07'));
        assert!(!clean.contains('\x00'));
        assert!(!clean.contains('\x08'));
        assert!(!clean.contains("[2J"));
        assert!(!clean.contains("[31m"));
        assert!(!clean.contains("0;you have been hacked"));
        assert!(!clean.contains("http://evil.example"));
        for c in clean.chars() {
            assert!(
                c == '\n' || c == '\t' || (c as u32) >= 0x20 && c as u32 != 0x7f,
                "control character {c:?} survived"
            );
        }

        // Text is kept — this is sanitizing, not dropping the reply.
        assert!(clean.starts_with("gate passed"));
        assert!(clean.contains("red "));
        assert!(clean.contains("link"));
        assert!(clean.contains("GATE FAILED"));
        assert!(clean.contains("\ttabbed"));
        assert!(clean.contains("\nnewline"));

        // And an ordinary reply is untouched.
        let plain = "Here is the plan.\n\n1. Read the spec\n2. Run the gate\n";
        assert_eq!(sanitize(plain), plain);
    }

    #[test]
    fn a_hostile_reply_cannot_reach_the_terminal_through_the_frame() {
        let mut c = Console::new(&["demo".to_string()]);
        c.complete_turn("demo", "ok\x1b[2J\x1b]0;title\x07", false);
        c.set_input("demo", "typed\x1b[31m");
        c.status = "note\x1b[H".to_string();
        let drawn = frame(&c, 80, 24);
        // The frame's own styling uses escapes, so assert on the payloads
        // the agent supplied rather than on the presence of any escape.
        assert!(!drawn.contains("[2J"));
        assert!(!drawn.contains("0;title"));
        assert!(!drawn.contains("[31m"));
        assert!(!drawn.contains("[H"));
        assert!(drawn.contains("ok"));
        assert!(drawn.contains("typed"));
    }

    #[test]
    fn wrap_breaks_long_lines_and_long_words() {
        let wrapped = wrap("aaa bbb ccc ddd", 7);
        assert!(wrapped.iter().all(|l| l.chars().count() <= 7));
        let long = wrap(&"x".repeat(50), 20);
        assert!(long.iter().all(|l| l.chars().count() <= 20));
        assert_eq!(long.concat(), "x".repeat(50));
    }

    /// The defect this replaced: a turn in flight showed one static phrase,
    /// in the project list at the top of the frame, eight lines from where
    /// the operator types — so a 110-second turn read as "no feedback at
    /// all, is it even busy?".
    #[test]
    fn a_turn_in_flight_shows_moving_elapsed_time_next_to_the_input_line() {
        use std::time::Duration;

        // Motion, so the console is visibly alive...
        let frames: Vec<char> = [0u64, 250, 500, 750]
            .iter()
            .map(|ms| {
                waiting_label(Duration::from_millis(*ms))
                    .chars()
                    .next()
                    .unwrap()
            })
            .collect();
        assert_eq!(frames.len(), 4);
        assert_eq!(
            frames
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4,
            "the label must change between redraws, or it reads as hung: {frames:?}"
        );

        // ...and elapsed time, so "how long" has an answer.
        assert!(waiting_label(Duration::from_secs(9)).contains("working 9s"));
        assert!(waiting_label(Duration::from_secs(110)).contains("working 1m50s"));
        assert!(waiting_label(Duration::from_secs(3600)).contains("working 60m00s"));

        // It appears in the body, immediately above the input line — not
        // only in the project list at the top.
        let mut c = Console::new(&["alpha".to_string()]);
        c.begin_turn("alpha");
        let drawn = frame(&c, 80, 24);
        let at_label = drawn
            .rfind("working")
            .expect("label is drawn near the input");
        let at_input = drawn.rfind("[alpha]").expect("input line is drawn");
        assert!(
            at_label < at_input,
            "the label must sit above the input line"
        );
        assert!(drawn.contains("keys still work"));
        // Two occurrences: the project row and the line above the input.
        assert_eq!(drawn.matches("working").count(), 2);

        // And it goes away when the turn lands.
        c.complete_turn("alpha", "the answer", false);
        let drawn = frame(&c, 80, 24);
        assert!(!drawn.contains("working"));
        assert!(drawn.contains("the answer"));
    }

    #[test]
    fn frame_marks_a_pending_project_and_names_an_armed_approval() {
        let mut c = Console::new(&["alpha".to_string(), "beta".to_string()]);
        c.begin_turn("alpha");
        let drawn = frame(&c, 80, 24);
        assert!(drawn.contains("working"));

        c.set_stage(
            "beta",
            Some("beta-spec".into()),
            Some("plan-approval".into()),
            Some("keel approve beta-spec --stage plan".into()),
        );
        c.selected = 1;
        c.arm_approval();
        let drawn = frame(&c, 80, 24);
        assert!(drawn.contains("beta-spec"));
        assert!(drawn.contains("'beta'"));
        assert!(drawn.contains("press y to confirm"));
    }
}
