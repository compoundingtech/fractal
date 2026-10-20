use crate::{Intent, Mission, StepState};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
pub use st3_conversation_ui::Theme;
pub use st3_conversation_ui::doc::Doc;
use st3_conversation_ui::doc::Target;
use st3_conversation_ui::text::{self, run};

/// Semantic colors. Defaults reproduce stui's Catppuccin Mocha palette.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub done: Color,
    pub fault: Color,
    pub person: Color,
    pub red: Color,
    pub sapphire: Color,
    pub surface0: Color,
    pub surface2: Color,
    pub waiting: Color,
    pub working: Color,
    pub text: Color,
    pub overlay0: Color,
    pub subtext0: Color,
}
impl Default for Palette {
    fn default() -> Self {
        Self {
            done: Color::Rgb(0x94, 0xe2, 0xd5),
            fault: Color::Rgb(0xf3, 0x8b, 0xa8),
            person: Color::Rgb(0xcb, 0xa6, 0xf7),
            red: Color::Rgb(0xf3, 0x8b, 0xa8),
            sapphire: Color::Rgb(0x74, 0xc7, 0xec),
            surface0: Color::Rgb(0x31, 0x32, 0x44),
            surface2: Color::Rgb(0x58, 0x5b, 0x70),
            waiting: Color::Rgb(0xf9, 0xe2, 0xaf),
            working: Color::Rgb(0xfa, 0xb3, 0x87),
            text: Color::Rgb(0xcd, 0xd6, 0xf4),
            overlay0: Color::Rgb(0x6c, 0x70, 0x86),
            subtext0: Color::Rgb(0xa6, 0xad, 0xc8),
        }
    }
}
impl Palette {
    fn text(self) -> Style {
        Style::default().fg(self.text)
    }
    fn dim(self) -> Style {
        Style::default().fg(self.overlay0)
    }
    fn soft(self) -> Style {
        Style::default().fg(self.subtext0)
    }
}

/// Glyph tokens; spinner frames remain caller-owned.
#[derive(Clone, Copy, Debug)]
pub struct Glyphs {
    pub done: &'static str,
    pub cancelled: &'static str,
    pub ready: &'static str,
    pub waiting: &'static str,
    pub needs_you: &'static str,
    pub failed: &'static str,
    pub pending: &'static str,
    pub expanded: &'static str,
    pub collapsed: &'static str,
    pub arrow: &'static str,
    pub eighths: [&'static str; 9],
}
impl Default for Glyphs {
    fn default() -> Self {
        Self {
            done: "▰",
            cancelled: "−",
            ready: "▱",
            waiting: "◐",
            needs_you: "◆",
            failed: "✕",
            pending: "▱",
            expanded: "▾ ",
            collapsed: "▸ ",
            arrow: " → ",
            eighths: [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"],
        }
    }
}
fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}
fn fg(color: Color) -> Style {
    Style::default().fg(color)
}
fn strong(color: Color) -> Style {
    fg(color).add_modifier(Modifier::BOLD)
}

pub fn step_style(
    state: StepState,
    spinner: &'static str,
    palette: &Palette,
    glyphs: &Glyphs,
) -> (&'static str, Color, &'static str) {
    match state {
        StepState::Done => (glyphs.done, palette.done, "done"),
        StepState::Cancelled => (glyphs.cancelled, palette.surface2, "cancelled"),
        StepState::Working => (spinner, palette.working, "working"),
        StepState::Ready => (glyphs.ready, palette.waiting, "ready"),
        StepState::Waiting => (glyphs.waiting, palette.sapphire, "waiting"),
        StepState::NeedsYou => (glyphs.needs_you, palette.person, "needs you"),
        StepState::Failed => (glyphs.failed, palette.fault, "failed"),
        StepState::Pending => (glyphs.pending, palette.surface2, "later"),
    }
}

/// Steps drawn as a flow: each layer after the one it depends on.
pub fn flow<'a>(
    steps: impl Iterator<Item = (&'a str, &'a [String], Color)>,
    width: usize,
    palette: &Palette,
    glyphs: &Glyphs,
) -> Vec<Line<'static>> {
    let steps = steps.collect::<Vec<_>>();
    let mut depth = vec![0usize; steps.len()];
    for _ in 0..steps.len() {
        for (index, (_, after, _)) in steps.iter().enumerate() {
            for dependency in after.iter() {
                if let Some(position) = steps.iter().position(|(name, _, _)| name == dependency) {
                    depth[index] = depth[index].max(depth[position] + 1);
                }
            }
        }
    }
    let layers = depth.iter().max().map(|max| max + 1).unwrap_or(0);
    let mut runs = vec![];
    for layer in 0..layers {
        if layer > 0 {
            runs.push(run(glyphs.arrow, fg(palette.surface2)));
        }
        let names = steps
            .iter()
            .enumerate()
            .filter(|(index, _)| depth[*index] == layer)
            .map(|(_, step)| *step)
            .collect::<Vec<_>>();
        // Steps that can run side by side read as a list: `a, b → c`.
        for (index, (name, _, color)) in names.iter().enumerate() {
            if index > 0 {
                runs.push(run(", ", palette.dim()));
            }
            runs.push(run(name.to_string(), fg(*color)));
        }
    }
    text::wrap(
        &runs,
        width,
        &[run("  ", palette.dim())],
        &[run("  ", palette.dim())],
        None,
    )
}

/// A meter of eighth blocks.
pub fn meter(
    done: usize,
    total: usize,
    cells: usize,
    color: Color,
    palette: &Palette,
    glyphs: &Glyphs,
) -> Vec<Span<'static>> {
    let filled = if total == 0 {
        0.0
    } else {
        done as f32 / total as f32 * cells as f32
    };
    (0..cells)
        .map(|cell| {
            let part = (filled - cell as f32).clamp(0.0, 1.0);
            Span::styled(
                glyphs.eighths[(part * 8.0).round() as usize],
                Style::default().fg(color).bg(palette.surface0),
            )
        })
        .collect()
}

/// Step rows and progress, without the shell's card frame.
/// Progress uses every described step, including cancelled and pending steps.
pub fn steps(
    mission: &Mission,
    inner: usize,
    spinner: &'static str,
    expanded: &std::collections::HashSet<String>,
    palette: &Palette,
    glyphs: &Glyphs,
    text_theme: &Theme,
) -> Doc<Intent> {
    let (done, total) = mission.progress();
    let mut steps = Doc::new();
    let mut bar = meter(done, total, 20, palette.done, palette, glyphs);
    bar.push(span(format!("  {done}/{total} done"), palette.dim()));
    steps.line(Line::from(bar));
    // The order only says something when a step waits for another.
    if mission.steps.iter().any(|step| !step.after.is_empty()) {
        steps.lines(flow(
            mission.steps.iter().map(|step| {
                (
                    step.name.as_str(),
                    step.after.as_slice(),
                    step_style(step.state, spinner, palette, glyphs).1,
                )
            }),
            inner,
            palette,
            glyphs,
        ));
    }
    let name_width = mission
        .steps
        .iter()
        .map(|step| text::width(&step.name))
        .max()
        .unwrap_or(8)
        .clamp(8, inner / 3);
    steps.blank();
    for step in &mission.steps {
        let (glyph, color, word) = step_style(step.state, spinner, palette, glyphs);
        let key = format!("step:{}:{}", mission.id, step.name);
        let open = expanded.contains(&key);
        steps.targets.push(Target {
            line: steps.lines.len(),
            column: 0,
            width: inner as u16,
            hit: Intent::ToggleStep(key),
        });
        steps.line(Line::from(vec![
            span(
                if open {
                    glyphs.expanded
                } else {
                    glyphs.collapsed
                },
                palette.dim(),
            ),
            span(format!("{glyph} "), strong(color)),
            span(
                format!(
                    "{:<w$} ",
                    text::truncate(&step.name, name_width),
                    w = name_width
                ),
                palette.text(),
            ),
            span(format!("{:<11}", word), fg(color)),
            span(
                // A long owner is cut so its age never runs into it.
                format!(
                    "{:<18}",
                    text::truncate(step.owner.as_deref().unwrap_or("nobody"), 17)
                ),
                if step.owner.is_some() {
                    palette.soft()
                } else {
                    fg(palette.waiting)
                },
            ),
            span(step.age.clone(), palette.dim()),
        ]));
        if let Some(note) = &step.note {
            steps.lines(text::wrap(
                &text::inline(note, palette.dim(), text_theme),
                inner,
                &[run("      ", palette.dim())],
                &[run("      ", palette.dim())],
                None,
            ));
        }
        // A step in hand says what it last reported, the status a person reads first.
        if step.state == StepState::Working
            && let Some(progress) = mission
                .step_metadata
                .get(&step.id)
                .and_then(|metadata| metadata.last_progress.as_deref())
        {
            steps.lines(text::wrap(
                &text::inline(progress, palette.text(), text_theme),
                inner,
                &[run("      ", palette.dim())],
                &[run("      ", palette.dim())],
                None,
            ));
        }
        // A step whose holder sat idle with nothing set to wake it says when st nudged it.
        if step.state == StepState::Working
            && let Some(nudged) = mission
                .step_metadata
                .get(&step.id)
                .and_then(|metadata| metadata.nudged.as_deref())
        {
            steps.line(Line::from(vec![
                span("      ", palette.dim()),
                span(
                    format!("nudged {nudged} ago: idle holding it, nothing set to wake it"),
                    palette.soft(),
                ),
            ]));
        }
        if open {
            let pad = "      ";
            let mut field = |label: &str, values: &[String], style: Style| {
                for (index, value) in values.iter().enumerate() {
                    let head = if index == 0 {
                        format!("{pad}{label:<12}")
                    } else {
                        format!("{pad}{:<12}", "")
                    };
                    steps.lines(text::wrap(
                        &text::inline(value, style, text_theme),
                        inner,
                        &[run(head, palette.dim())],
                        &[run(format!("{pad}{:<12}", ""), palette.dim())],
                        None,
                    ));
                }
            };
            field("goal", &step.goals, palette.text());
            field("constraint", &step.constraints, palette.soft());
            field("gate", &step.gates, fg(palette.person));
            field("blocked by", &step.blockers, fg(palette.red));
            if !step.after.is_empty() {
                field("after", &[step.after.join(", ")], palette.soft());
            }
            field("attempt", &[step.attempt.to_string()], palette.soft());
            steps.blank();
        }
    }
    steps.line(Line::from(span("click a step to open it", palette.dim())));
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_uses_caller_tokens_and_empty_denominators() {
        let palette = Palette {
            surface0: Color::Blue,
            ..Palette::default()
        };
        let glyphs = Glyphs {
            eighths: ["0", "1", "2", "3", "4", "5", "6", "7", "8"],
            ..Glyphs::default()
        };
        let empty = meter(0, 0, 2, Color::Red, &palette, &glyphs);
        assert_eq!(
            empty
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "00"
        );
        assert_eq!(
            empty[0].style,
            Style::default().fg(Color::Red).bg(Color::Blue)
        );
        let half = meter(1, 2, 1, Color::Red, &palette, &glyphs);
        assert_eq!(half[0].content, "4");
    }

    #[test]
    fn working_style_uses_the_callers_spinner() {
        let palette = Palette {
            working: Color::Magenta,
            ..Palette::default()
        };
        assert_eq!(
            step_style(StepState::Working, "*", &palette, &Glyphs::default()),
            ("*", Color::Magenta, "working")
        );
    }
}
