//! Predicate parsing shared by `when` and `requires`.
//!
//! Semantics:
//! - each predicate list is an AND over items
//! - `!` negates a single predicate atom
//! - `when` and `requires` share syntax but differ in intent

use std::fmt;

use crate::env_name::validate_env_var_name;
use crate::error::ConchError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    pub negated: bool,
    pub atom: PredicateAtom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicateAtom {
    Interactive,
    Login,
    Shell(String),
    Command(String),
    EnvExists(String),
    EnvEquals { name: String, value: String },
    File(String),
    Dir(String),
    Os(String),
    Hostname(String),
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negated {
            f.write_str("!")?;
        }
        match &self.atom {
            PredicateAtom::Interactive => f.write_str("interactive"),
            PredicateAtom::Login => f.write_str("login"),
            PredicateAtom::Shell(name) => write!(f, "shell:{name}"),
            PredicateAtom::Command(name) => write!(f, "command:{name}"),
            PredicateAtom::EnvExists(name) => write!(f, "env:{name}"),
            PredicateAtom::EnvEquals { name, value } => write!(f, "env:{name}={value}"),
            PredicateAtom::File(path) => write!(f, "file:{path}"),
            PredicateAtom::Dir(path) => write!(f, "dir:{path}"),
            PredicateAtom::Os(name) => write!(f, "os:{name}"),
            PredicateAtom::Hostname(name) => write!(f, "hostname:{name}"),
        }
    }
}

impl Predicate {
    pub fn parse(input: &str) -> Result<Self, ConchError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(predicate_error(input, "predicate cannot be empty"));
        }

        let (negated, body) = if let Some(rest) = trimmed.strip_prefix('!') {
            if rest.is_empty() {
                return Err(predicate_error(
                    input,
                    "`!` must be followed by a predicate, for example `!interactive` or `!command:nvim`",
                ));
            }
            (true, rest)
        } else {
            (false, trimmed)
        };

        let atom = parse_atom(body, input)?;
        Ok(Self { negated, atom })
    }
}

fn parse_atom(body: &str, original: &str) -> Result<PredicateAtom, ConchError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(predicate_error(
            original,
            "expected `interactive`, `login`, or `<kind>:<value>`",
        ));
    }

    match body {
        "interactive" => return Ok(PredicateAtom::Interactive),
        "login" => return Ok(PredicateAtom::Login),
        _ => {}
    }

    let Some((kind, value)) = body.split_once(':') else {
        return Err(predicate_error(
            original,
            "expected `interactive`, `login`, or `<kind>:<value>`",
        ));
    };

    let kind = kind.trim();
    if kind.is_empty() {
        return Err(predicate_error(
            original,
            "predicate kind cannot be empty; expected `interactive`, `login`, or `<kind>:<value>`",
        ));
    }

    let value = value.trim_start();
    if value.is_empty() {
        return Err(predicate_error(
            original,
            &format!("predicate kind `{kind}` requires a value, for example `{kind}:...`"),
        ));
    }

    match kind {
        "shell" => Ok(PredicateAtom::Shell(value.to_string())),
        "command" => Ok(PredicateAtom::Command(value.to_string())),
        "env" => {
            if let Some((name, expected)) = value.split_once('=') {
                let name = name.trim();
                if name.is_empty() {
                    return Err(predicate_error(
                        original,
                        "`env:` predicates must include a variable name before `=`",
                    ));
                }
                validate_env_var_name(name, original)?;
                Ok(PredicateAtom::EnvEquals {
                    name: name.to_string(),
                    value: expected.to_string(),
                })
            } else {
                let name = value.trim();
                validate_env_var_name(name, original)?;
                Ok(PredicateAtom::EnvExists(name.to_string()))
            }
        }
        "file" => {
            crate::provider::subst::parse_interpolated_value(value).map_err(|err| {
                let detail = err.to_string();
                let detail = detail
                    .strip_prefix("invalid configuration: ")
                    .unwrap_or(&detail);
                predicate_error(original, detail)
            })?;
            Ok(PredicateAtom::File(value.to_string()))
        }
        "dir" => {
            crate::provider::subst::parse_interpolated_value(value).map_err(|err| {
                let detail = err.to_string();
                let detail = detail
                    .strip_prefix("invalid configuration: ")
                    .unwrap_or(&detail);
                predicate_error(original, detail)
            })?;
            Ok(PredicateAtom::Dir(value.to_string()))
        }
        "os" => Ok(PredicateAtom::Os(value.to_string())),
        "hostname" => Ok(PredicateAtom::Hostname(value.to_string())),
        _ => Err(predicate_error(
            original,
            &format!(
                "unsupported predicate kind `{kind}`; supported kinds are `shell`, `command`, `env`, `file`, `dir`, `os`, and `hostname`"
            ),
        )),
    }
}

fn predicate_error(input: &str, reason: &str) -> ConchError {
    ConchError::PredicateParse(format!("`{input}`: {reason}"))
}

pub fn parse_predicates(values: &[String]) -> Result<Vec<Predicate>, ConchError> {
    values.iter().map(|value| Predicate::parse(value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_predicates_for_display() {
        assert_eq!(
            Predicate {
                negated: true,
                atom: PredicateAtom::Shell("fish".into()),
            }
            .to_string(),
            "!shell:fish"
        );
        assert_eq!(
            Predicate {
                negated: false,
                atom: PredicateAtom::EnvEquals {
                    name: "A".into(),
                    value: "b".into(),
                },
            }
            .to_string(),
            "env:A=b"
        );
    }

    #[test]
    fn parses_predicates() {
        assert_eq!(
            Predicate::parse("interactive").unwrap(),
            Predicate {
                negated: false,
                atom: PredicateAtom::Interactive,
            }
        );
        assert_eq!(
            Predicate::parse("env:  PATH").unwrap(),
            Predicate {
                negated: false,
                atom: PredicateAtom::EnvExists("PATH".into()),
            }
        );
        assert_eq!(
            Predicate::parse("!env:EDITOR=nvim").unwrap(),
            Predicate {
                negated: true,
                atom: PredicateAtom::EnvEquals {
                    name: "EDITOR".into(),
                    value: "nvim".into(),
                },
            }
        );
        assert_eq!(
            Predicate::parse("dir:~/.config").unwrap(),
            Predicate {
                negated: false,
                atom: PredicateAtom::Dir("~/.config".into()),
            }
        );
        assert_eq!(
            Predicate::parse("shell :fish").unwrap(),
            Predicate {
                negated: false,
                atom: PredicateAtom::Shell("fish".into()),
            }
        );
        assert_eq!(
            Predicate::parse("! interactive").unwrap(),
            Predicate {
                negated: true,
                atom: PredicateAtom::Interactive,
            }
        );
        assert_eq!(
            Predicate::parse("! login").unwrap(),
            Predicate {
                negated: true,
                atom: PredicateAtom::Login,
            }
        );
    }

    #[test]
    fn rejects_invalid_predicates_with_context() {
        let cases = [
            "",
            "!",
            "shell",
            "shell:",
            ":nope",
            "env:=value",
            "env:   ",
            "env:9BAD",
            "env:MY-VAR",
            "env:MY VAR",
            "nope:value",
        ];

        for case in cases {
            let err = Predicate::parse(case).unwrap_err();
            let rendered = err.to_string();
            assert!(
                rendered.contains("invalid configuration")
                    || rendered.contains("invalid predicate"),
                "{rendered}"
            );
            assert!(rendered.contains(&format!("`{case}`")));
        }
    }
}
