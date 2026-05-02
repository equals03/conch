//! Rules for environment variable identifiers shared by `env:` predicates and `${env:...}` interpolation.

use crate::error::ConchError;

pub(crate) fn validate_env_var_name(name: &str, context: &str) -> Result<(), ConchError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(ConchError::Validation(format!(
            "`{context}`: `env:` variable name is empty, for example `env:EDITOR` or `${{env:HOME}}`"
        )));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(ConchError::Validation(format!(
            "`{context}`: `env:` / `${{env:...}}` names must start with an ASCII letter or underscore"
        )));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ConchError::Validation(format!(
            "`{context}`: `env:` / `${{env:...}}` names may contain only ASCII letters, digits, and underscores"
        )));
    }
    Ok(())
}
