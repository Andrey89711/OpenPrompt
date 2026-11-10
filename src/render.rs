use crate::{
    config::{color_code, Config, Layout, Theme},
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
    pub extra: &'a [String],
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
    kind: &'static str,
    text: String,
    foreground: u8,
}

pub fn render(config: &Config, context: &Context<'_>) -> String {
    let (symbol, symbol_color) = if context.status == 0 {
        (
            &config.prompt.success_symbol,
            color_code(&config.colors.success).unwrap_or(32),
        )
    } else {
        (
            &config.prompt.error_symbol,
            color_code(&config.colors.error).unwrap_or(31),
        )
    };
    let symbol = if config.icons && symbol == ">" {
        "❯"
    } else {
        symbol.as_str()
    };
    let one_line = config.prompt.layout == Layout::OneLine
        || (config.prompt.layout == Layout::Auto && config.theme == Theme::Minimal);
    let width = context.width.clamp(10, 1000);
    let budget = width.saturating_sub(if one_line {
        UnicodeWidthStr::width(symbol) + 3
    } else {
        1
    });
    let mut segments = Vec::new();
    if config.directory.enabled {
        let path = directory(
            context.path,
            context.home,
            config.directory.max_width.min(budget),
        );
        segments.push(Segment {
            kind: "directory",
            text: if config.icons {
                format!("\u{f07b} {path}")
            } else {
                path
            },
            foreground: color_code(&config.colors.directory).unwrap_or(36),
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
            if git.staged > 0 {
                text.push_str(&format!(" staged:{}", git.staged));
            }
            if git.modified > 0 {
                text.push_str(&format!(" modified:{}", git.modified));
            }
            if git.untracked > 0 {
                text.push_str(&format!(" new:{}", git.untracked));
            }
            if git.conflicts > 0 {
                text.push_str(&format!(" conflicts:{}", git.conflicts));
            }
            if git.ahead > 0 {
                text.push_str(&format!(" +{}", git.ahead));
            }
            if git.behind > 0 {
                text.push_str(&format!(" -{}", git.behind));
            }
            segments.push(Segment {
                kind: "git",
                text,
                foreground: color_code(&config.colors.git).unwrap_or(35),
            });
        }
    }
    if config.duration.enabled && context.duration_ms >= config.duration.min_ms {
        segments.push(Segment {
            kind: "duration",
            text: duration(context.duration_ms),
            foreground: color_code(&config.colors.duration).unwrap_or(33),
        });
    }
    if config.status.enabled && context.status != 0 {
        segments.push(Segment {
            kind: "status",
            text: format!("exit:{}", context.status),
            foreground: color_code(&config.colors.status).unwrap_or(31),
        });
    }
    for extra in context.extra {
        let (kind, color) = if extra.starts_with("venv:")
            || extra.starts_with("conda:")
            || extra.starts_with("env:")
        {
            ("environment", &config.colors.environment)
        } else if extra == "ssh" || extra == "admin" {
            ("session", &config.colors.session)
        } else {
            ("runtime", &config.colors.runtime)
        };
        segments.push(Segment {
            kind,
            text: sanitize(extra),
            foreground: color_code(color).unwrap_or(34),
        });
    }
    segments.retain(|segment| {
        config
            .segments
            .order
            .iter()
            .any(|name| name == segment.kind)
    });
    segments.sort_by_key(|segment| {
        config
            .segments
            .order
            .iter()
            .position(|name| name == segment.kind)
            .unwrap_or(usize::MAX)
    });
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
                let foreground = if [32, 33, 36, 37, 92, 93, 96, 97].contains(&segment.foreground) {
                    30
                } else {
                    97
                };
                info.push_str(&format!(
                    "\x1b[{};{}m {} \x1b[0m",
                    foreground,
                    segment.foreground + 10,
                    text
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
    let symbol = if context.color {
        format!("\x1b[{symbol_color}m{symbol}\x1b[0m ")
    } else {
        format!("{symbol} ")
    };
    if info.is_empty() {
        return symbol;
    }
    if one_line {
        format!("{info} {symbol}")
    } else {
        format!("{info}\n{symbol}")
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
            extra: &[],
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
            ..GitStatus::default()
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

    #[test]
    fn custom_order_symbols_colors_and_long_symbol_width_are_respected() {
        let config = crate::config::parse("theme = 'minimal'\n[prompt]\nsuccess_symbol = '12345678'\n[segments]\norder = ['status', 'directory']\n[colors]\ndirectory = 'bright-cyan'").unwrap();
        let mut ctx = context(Path::new("long-directory-name"));
        ctx.width = 20;
        let output = render(&config, &ctx);
        assert!(output.ends_with("12345678 "));
        assert!(UnicodeWidthStr::width(output.as_str()) < 20);
        ctx.color = true;
        assert!(render(&config, &ctx).contains("\x1b[96m"));
    }

    #[test]
    fn extra_segment_text_is_sanitized_and_ordered() {
        let extras = vec![
            "env:demo\x1b[31m".into(),
            "node:v22.0.0".into(),
            "ssh".into(),
        ];
        let mut ctx = context(Path::new("project"));
        ctx.extra = &extras;
        let output = render(&Config::default(), &ctx);
        assert!(output.starts_with("ssh  project"));
        assert!(!output.contains('\x1b'));
        assert!(output.contains("env:demo"));
    }
}
