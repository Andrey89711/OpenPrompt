use crate::{
    config::{Config, Theme},
    git::GitStatus,
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

pub struct Context<'a> {
    pub path: &'a Path,
    pub home: Option<&'a Path>,
    pub status: i64,
    pub duration_ms: u64,
    pub width: usize,
    pub color: bool,
    pub git: Option<&'a GitStatus>,
}

// Never let repository names or directory names inject terminal commands,
// line breaks, or bidirectional control characters into a prompt.
pub fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() && !matches!(c,
        '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'
    )).collect()
}

fn shorten(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut tail = String::new();
    let mut used = 1;
    for ch in text.chars().rev() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        tail.push(ch);
        used += w;
    }
    format!("…{}", tail.chars().rev().collect::<String>())
}

fn directory(path: &Path, home: Option<&Path>, max: usize) -> String {
    let display = if let Some(relative) = home.and_then(|home| path.strip_prefix(home).ok()) {
        if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~{}{}", std::path::MAIN_SEPARATOR, relative.display())
        }
    } else {
        path.to_string_lossy().into_owned()
    };
    shorten(&sanitize(&display), max)
}

fn duration(ms: u64) -> String {
    if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {}s", ms / 60_000, ms % 60_000 / 1000)
    }
}

struct Segment {
    text: String,
    foreground: &'static str,
    background: &'static str,
}

pub fn render(config: &Config, context: &Context<'_>) -> String {
    let budget = context.width.clamp(10, 1000).saturating_sub(4);
    let mut segments = Vec::new();
    if config.directory.enabled {
        let path = directory(
            context.path,
            context.home,
            config.directory.max_width.min(budget),
        );
        segments.push(Segment {
            text: if config.icons {
                format!("\u{f07b} {path}")
            } else {
                path
            },
            foreground: "36",
            background: "44",
        });
    }
    if config.git.enabled {
        if let Some(git) = context.git {
            let branch = shorten(&sanitize(&git.branch), 24);
            let mut text = if config.icons {
                format!("\u{e0a0} {branch}")
            } else {
                format!("git:{branch}")
            };
            if git.dirty {
                text.push('*');
            }
            if git.ahead > 0 {
                text.push_str(&format!(" +{}", git.ahead));
            }
            if git.behind > 0 {
                text.push_str(&format!(" -{}", git.behind));
            }
            segments.push(Segment {
                text,
                foreground: "35",
                background: "45",
            });
        }
    }
    if config.duration.enabled && context.duration_ms >= config.duration.min_ms {
        segments.push(Segment {
            text: duration(context.duration_ms),
            foreground: "33",
            background: "43",
        });
    }
    if config.status.enabled && context.status != 0 {
        segments.push(Segment {
            text: format!("exit:{}", context.status),
            foreground: "31",
            background: "41",
        });
    }
    let mut info = String::new();
    let mut used = 0;
    for segment in segments {
        let padding = if config.theme == Theme::Blocks || used > 0 {
            2
        } else {
            0
        };
        let remaining = budget.saturating_sub(used + padding);
        if remaining < 2 {
            break;
        }
        let text = shorten(&segment.text, remaining);
        let text_width = UnicodeWidthStr::width(text.as_str());
        if text_width == 0 {
            continue;
        }
        if config.theme == Theme::Blocks {
            if context.color {
                let foreground = if segment.background == "43" {
                    "30"
                } else {
                    "97"
                };
                info.push_str(&format!(
                    "\x1b[{};{}m {} \x1b[0m",
                    foreground, segment.background, text
                ));
            } else {
                info.push_str(&format!(" {text} "));
            }
        } else {
            if used > 0 {
                info.push_str("  ");
            }
            if context.color {
                info.push_str(&format!("\x1b[{}m{}\x1b[0m", segment.foreground, text));
            } else {
                info.push_str(&text);
            }
        }
        used += padding + text_width;
    }
    let symbol = if config.icons { "❯" } else { ">" };
    let symbol_color = if context.status == 0 { "32" } else { "31" };
    let symbol = if context.color {
        format!("\x1b[{symbol_color}m{symbol}\x1b[0m ")
    } else {
        format!("{symbol} ")
    };
    if info.is_empty() {
        return symbol;
    }
    match config.theme {
        Theme::Minimal => format!("{info} {symbol}"),
        Theme::Classic | Theme::Blocks => format!("{info}\n{symbol}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(path: &Path) -> Context<'_> {
        Context {
            path,
            home: None,
            status: 0,
            duration_ms: 0,
            width: 80,
            color: false,
            git: None,
        }
    }

    #[test]
    fn classic_is_two_lines_and_no_trailing_newline() {
        assert_eq!(
            render(&Config::default(), &context(Path::new("project"))),
            "project\n> "
        );
    }

    #[test]
    fn home_is_replaced_only_at_path_boundary() {
        assert_eq!(
            directory(Path::new("/home/demo"), Some(Path::new("/home/demo")), 40),
            "~"
        );
        assert_eq!(
            directory(
                Path::new("/home/demolition"),
                Some(Path::new("/home/demo")),
                40
            ),
            "/home/demolition"
        );
    }

    #[test]
    fn status_git_duration_and_plain_output() {
        let git = GitStatus {
            branch: "main".into(),
            dirty: true,
            ahead: 2,
            behind: 1,
        };
        let mut ctx = context(Path::new("project"));
        ctx.git = Some(&git);
        ctx.status = 42;
        ctx.duration_ms = 2500;
        let output = render(&Config::default(), &ctx);
        assert!(output.contains("git:main* +2 -1"));
        assert!(output.contains("2.5s"));
        assert!(output.contains("exit:42"));
        assert!(!output.contains('\x1b'));
    }

    #[test]
    fn untrusted_names_cannot_inject_terminal_controls() {
        assert_eq!(
            sanitize("bad\x1b]0;title\x07\n\r\u{202e}path"),
            "bad]0;titlepath"
        );
        let output = render(&Config::default(), &context(Path::new("folder\n\x1b[31m")));
        assert_eq!(output.lines().count(), 2);
        assert!(!output.contains('\x1b'));
    }

    #[test]
    fn unicode_paths_fit_narrow_windows() {
        let mut ctx = context(Path::new("очень-длинная-папка/项目/😀/project"));
        ctx.width = 20;
        for theme in [Theme::Classic, Theme::Minimal, Theme::Blocks] {
            let config = Config {
                theme,
                ..Config::default()
            };
            let output = render(&config, &ctx);
            for line in output.lines() {
                assert!(UnicodeWidthStr::width(line) < 20, "{line}");
            }
        }
    }

    #[test]
    fn colors_are_reset_and_segments_can_be_disabled() {
        let mut ctx = context(Path::new("project"));
        ctx.color = true;
        let mut config = Config::default();
        assert!(render(&config, &ctx).contains("\x1b[0m"));
        config.directory.enabled = false;
        assert_eq!(render(&config, &context(Path::new("project"))), "> ");
    }
}
