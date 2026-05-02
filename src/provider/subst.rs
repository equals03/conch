//! `${env:NAME}` substitution and leading `~` / `~/` (current-user home) in interpolated strings.
//!
//! Values are validated at config load; providers assume parse success when rendering.
//!
//! Shell-facing interpolation supports `\` escapes (see [`parse_interpolated_value`]). Paths used
//! in build-time [`crate::build`] `file:` / `dir:` folding use [`parse_predicate_path_interp`]
//! instead so native Windows separators are not swallowed.

use crate::env_name::validate_env_var_name;
use crate::error::ConchError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InterpSegment {
    Lit(String),
    Env(String),
    /// `~` or the prefix of `~/…` — rendered as `$HOME` for v1 POSIX-style shells.
    Home,
}

pub(crate) fn parse_interpolated_value(input: &str) -> Result<Vec<InterpSegment>, ConchError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    if input == "~" {
        segments.push(InterpSegment::Home);
        return Ok(segments);
    }

    if let Some(rest) = input.strip_prefix("~/") {
        segments.push(InterpSegment::Home);
        if rest.is_empty() {
            return Ok(segments);
        }
        let tail = format!("/{rest}");
        parse_tail(&tail, input, &mut segments)?;
        return Ok(segments);
    }

    parse_tail(input, input, &mut segments)?;
    Ok(segments)
}

/// Like [`parse_interpolated_value`], but backslashes are kept for normal path characters (Windows).
/// The only special case is `\$` → a literal `$`, so values like `\${env:HOME}` remain a literal for
/// config authors without breaking `C:\Users\...\file`.
pub(crate) fn parse_predicate_path_interp(input: &str) -> Result<Vec<InterpSegment>, ConchError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    if input == "~" {
        segments.push(InterpSegment::Home);
        return Ok(segments);
    }

    if let Some(rest) = input.strip_prefix("~/") {
        segments.push(InterpSegment::Home);
        if rest.is_empty() {
            return Ok(segments);
        }
        let tail = format!("/{rest}");
        parse_tail_predicate_path(&tail, input, &mut segments)?;
        return Ok(segments);
    }

    parse_tail_predicate_path(input, input, &mut segments)?;
    Ok(segments)
}

fn parse_tail_predicate_path(
    tail: &str,
    full_input: &str,
    segments: &mut Vec<InterpSegment>,
) -> Result<(), ConchError> {
    let mut lit = String::new();
    let mut iter = tail.char_indices().peekable();

    while let Some((_, c)) = iter.next() {
        if c == '\\' && iter.peek().is_some_and(|&(_, ch)| ch == '$') {
            iter.next(); // consume '$'
            lit.push('$');
            continue;
        }

        if c == '$' && iter.peek().is_some_and(|&(_, ch)| ch == '{') {
            iter.next(); // consume '{'
            flush_lit(segments, &mut lit);
            parse_env_brace(&mut iter, full_input, segments)?;
            continue;
        }

        lit.push(c);
    }

    flush_lit(segments, &mut lit);
    Ok(())
}

fn parse_tail(
    tail: &str,
    full_input: &str,
    segments: &mut Vec<InterpSegment>,
) -> Result<(), ConchError> {
    let mut lit = String::new();
    let mut iter = tail.char_indices().peekable();

    while let Some((_, c)) = iter.next() {
        if c == '\\' {
            let Some((_, next)) = iter.next() else {
                return Err(ConchError::Validation(format!(
                    "invalid `\\` escape at end of interpolated value `{full_input}`"
                )));
            };
            lit.push(next);
            continue;
        }

        if c == '$' && iter.peek().is_some_and(|&(_, ch)| ch == '{') {
            iter.next(); // consume '{'
            flush_lit(segments, &mut lit);
            parse_env_brace(&mut iter, full_input, segments)?;
            continue;
        }

        lit.push(c);
    }

    flush_lit(segments, &mut lit);
    Ok(())
}

fn flush_lit(segments: &mut Vec<InterpSegment>, lit: &mut String) {
    if lit.is_empty() {
        return;
    }
    segments.push(InterpSegment::Lit(std::mem::take(lit)));
}

fn parse_env_brace(
    iter: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    full_input: &str,
    segments: &mut Vec<InterpSegment>,
) -> Result<(), ConchError> {
    const PREFIX: &str = "env:";
    for exp in PREFIX.chars() {
        let Some((_, ch)) = iter.next() else {
            return Err(ConchError::Validation(format!(
                "unfinished `${{env:...}}` in `{full_input}`"
            )));
        };
        if ch != exp {
            return Err(ConchError::Validation(format!(
                "unsupported `${{...}}` in `{full_input}`; only `${{env:VAR}}` is allowed (VAR matches `env:` predicate names)"
            )));
        }
    }

    let mut name = String::new();
    loop {
        match iter.next() {
            Some((_, '}')) => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    return Err(ConchError::Validation(format!(
                        "`${{env:}}` in `{full_input}` must name a variable, for example `${{env:HOME}}`"
                    )));
                }
                validate_env_var_name(trimmed, full_input)?;
                segments.push(InterpSegment::Env(trimmed.to_string()));
                return Ok(());
            }
            Some((_, ch)) => name.push(ch),
            None => {
                return Err(ConchError::Validation(format!(
                    "missing `}}` to close `${{env:{name}` in `{full_input}`"
                )));
            }
        }
    }
}

fn bash_escape_literal_in_dquote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
        .replace('!', "\\!")
}

fn fish_escape_literal_in_dquote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Double-quoted bash RHS with parameter expansion for `${env:...}` / `$HOME` home segments.
pub(crate) fn bash_render_interpolated(segments: &[InterpSegment]) -> String {
    if segments.is_empty() {
        return "\"\"".to_string();
    }
    let mut out = String::from("\"");
    for seg in segments {
        match seg {
            InterpSegment::Lit(s) => out.push_str(&bash_escape_literal_in_dquote(s)),
            InterpSegment::Env(name) => {
                out.push_str("${");
                out.push_str(name);
                out.push('}');
            }
            InterpSegment::Home => out.push_str("$HOME"),
        }
    }
    out.push('"');
    out
}

/// Double-quoted fish value with `$VAR` expansion for env segments and `$HOME` for home.
pub(crate) fn fish_render_interpolated(segments: &[InterpSegment]) -> String {
    if segments.is_empty() {
        return "\"\"".to_string();
    }
    let mut out = String::from("\"");
    for seg in segments {
        match seg {
            InterpSegment::Lit(s) => out.push_str(&fish_escape_literal_in_dquote(s)),
            InterpSegment::Env(name) => {
                out.push('$');
                out.push_str(name);
            }
            InterpSegment::Home => out.push_str("$HOME"),
        }
    }
    out.push('"');
    out
}

/// Bash path fragments: like env interpolation but omits `!` history expansion in literals.
pub(crate) fn bash_render_path_interpolated(segments: &[InterpSegment]) -> String {
    if segments.is_empty() {
        return "\"\"".to_string();
    }
    let mut out = String::from("\"");
    for seg in segments {
        match seg {
            InterpSegment::Lit(s) => {
                let escaped = s
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('$', "\\$")
                    .replace('`', "\\`");
                out.push_str(&escaped);
            }
            InterpSegment::Env(name) => {
                out.push_str("${");
                out.push_str(name);
                out.push('}');
            }
            InterpSegment::Home => out.push_str("$HOME"),
        }
    }
    out.push('"');
    out
}

fn fish_escape_path_literal_in_dquote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

pub(crate) fn fish_render_path_interpolated(segments: &[InterpSegment]) -> String {
    if segments.is_empty() {
        return "\"\"".to_string();
    }
    let mut out = String::from("\"");
    for seg in segments {
        match seg {
            InterpSegment::Lit(s) => out.push_str(&fish_escape_path_literal_in_dquote(s)),
            InterpSegment::Env(name) => {
                out.push('$');
                out.push_str(name);
            }
            InterpSegment::Home => out.push_str("$HOME"),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_leading_tilde_only() {
        assert_eq!(
            parse_interpolated_value("~").unwrap(),
            vec![InterpSegment::Home]
        );
    }

    #[test]
    fn parses_tilde_slash_and_env() {
        assert_eq!(
            parse_interpolated_value("~/.local/${env:XDG_DATA_HOME}/share").unwrap(),
            vec![
                InterpSegment::Home,
                InterpSegment::Lit("/.local/".into()),
                InterpSegment::Env("XDG_DATA_HOME".into()),
                InterpSegment::Lit("/share".into()),
            ]
        );
    }

    #[test]
    fn rejects_unknown_brace_form() {
        let err = parse_interpolated_value("${foo:bar}").unwrap_err();
        assert!(err.to_string().contains("only `${env:VAR}`"));
    }

    #[test]
    fn backslash_escapes_dollar_brace() {
        assert_eq!(
            parse_interpolated_value(r"\${env:HOME}").unwrap(),
            vec![InterpSegment::Lit("${env:HOME}".into())]
        );
    }

    #[test]
    fn predicate_path_preserves_windows_separators() {
        assert_eq!(
            parse_predicate_path_interp(r"C:\tmp\predicates\x\cfg").unwrap(),
            vec![InterpSegment::Lit(r"C:\tmp\predicates\x\cfg".into())]
        );
    }

    #[test]
    fn predicate_path_dollar_brace_escape_matches_shell_interp() {
        assert_eq!(
            parse_predicate_path_interp(r"\${env:HOME}").unwrap(),
            parse_interpolated_value(r"\${env:HOME}").unwrap(),
        );
    }
}
