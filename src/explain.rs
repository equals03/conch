//! Human-readable explain output for resolved configurations.
//!
//! This module renders shell-neutral resolution details. It does not perform
//! graph construction, predicate evaluation, or merging on its own.

use std::collections::HashMap;
use std::fmt::Write;

use crate::ir::PathOp;
use crate::resolve::{BindingReport, BlockReport, Resolution};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderOptions {
    pub color: bool,
}

pub fn render_resolution(resolution: &Resolution, options: RenderOptions) -> String {
    let mut out = String::new();
    let style = Style {
        color: options.color,
    };
    let shell = match resolution.target_shell.as_str() {
        "" => "unknown",
        name => name,
    };
    let contributions = block_contributions(resolution);

    writeln!(out, "{} {}", style.bold("conch explain"), style.cyan(shell)).unwrap();
    out.push('\n');

    render_section_title(&mut out, &style, "Block order");
    if resolution.block_order.is_empty() {
        render_empty_line(&mut out);
    } else {
        for (index, block_id) in resolution.block_order.iter().enumerate() {
            writeln!(out, "  {:>2}. {}", index + 1, style.bold(block_id)).unwrap();
        }
    }
    out.push('\n');

    render_section_title(&mut out, &style, "Blocks");
    if resolution.block_reports.is_empty() {
        render_empty_line(&mut out);
    } else {
        for report in &resolution.block_reports {
            render_block_record(
                &mut out,
                report,
                contributions
                    .get(&report.block_id)
                    .map_or(&[][..], Vec::as_slice),
                &style,
            );
        }
    }
    out.push('\n');

    render_section_title(&mut out, &style, "Env bindings");
    render_binding_group(&mut out, &resolution.env_bindings, "env", &style);
    out.push('\n');

    render_section_title(&mut out, &style, "Alias bindings");
    render_binding_group(&mut out, &resolution.alias_bindings, "alias", &style);
    out.push('\n');

    render_section_title(&mut out, &style, "PATH timeline");
    if resolution.path_ops.is_empty() {
        render_empty_line(&mut out);
    } else {
        for (index, op) in resolution.path_ops.iter().enumerate() {
            writeln!(
                out,
                "  {:>2}. {}  {}",
                index + 1,
                style.bold(&op.block_id),
                describe_path_op(&op.op)
            )
            .unwrap();
        }
    }

    out
}

fn render_block_record(
    out: &mut String,
    report: &BlockReport,
    contributions: &[String],
    style: &Style,
) {
    writeln!(
        out,
        "  {}  {}",
        style.bold(&report.block_id),
        if report.guarded {
            style.yellow("guarded")
        } else {
            style.dim("unguarded")
        },
    )
    .unwrap();
    render_labeled_value(out, style, "when", &summarize_list(&report.when));
    render_labeled_value(out, style, "requires", &summarize_list(&report.requires));
    render_labeled_value(out, style, "contributes", &summarize_list(contributions));
}

fn render_labeled_value(out: &mut String, style: &Style, label: &str, value: &str) {
    writeln!(out, "    {:<11} {}", style.dim(label), value).unwrap();
}

fn render_section_title(out: &mut String, style: &Style, title: &str) {
    writeln!(out, "{}", style.bold(title)).unwrap();
}

fn render_empty_line(out: &mut String) {
    out.push_str("  (none)\n");
}

fn summarize_list(items: &[String]) -> String {
    if items.is_empty() {
        "-".into()
    } else {
        items.join(", ")
    }
}

fn push_binding_writes(
    contributions: &mut HashMap<String, Vec<String>>,
    bindings: &[BindingReport],
    kind: &str,
) {
    for binding in bindings {
        for writer in &binding.writers {
            contributions
                .entry(writer.block_id.clone())
                .or_default()
                .push(format!(
                    "{kind} {}={}",
                    binding.key,
                    writer.value.describe()
                ));
        }
    }
}

fn block_contributions(resolution: &Resolution) -> HashMap<String, Vec<String>> {
    let mut contributions: HashMap<String, Vec<String>> = HashMap::new();

    push_binding_writes(&mut contributions, &resolution.env_bindings, "env");
    push_binding_writes(&mut contributions, &resolution.alias_bindings, "alias");

    for path in &resolution.path_ops {
        contributions
            .entry(path.block_id.clone())
            .or_default()
            .push(format!("PATH {}", describe_path_op(&path.op)));
    }

    for report in &resolution.block_reports {
        if report.source_line_count > 0 {
            contributions
                .entry(report.block_id.clone())
                .or_default()
                .push(format!(
                    "source {} verbatim {}",
                    report.source_line_count,
                    if report.source_line_count == 1 {
                        "line"
                    } else {
                        "lines"
                    }
                ));
        }
    }

    contributions
}

fn render_binding_group(out: &mut String, bindings: &[BindingReport], kind: &str, style: &Style) {
    if bindings.is_empty() {
        render_empty_line(out);
        return;
    }

    for binding in bindings {
        writeln!(out, "  {} {}", kind, style.bold(&binding.key)).unwrap();
        writeln!(
            out,
            "    {:<11} {}",
            style.dim("writers"),
            binding_chain(binding, style)
        )
        .unwrap();
    }
}

fn binding_chain(binding: &BindingReport, style: &Style) -> String {
    let writers = &binding.writers;
    let last_index = writers.len().saturating_sub(1);
    let mut out = String::new();
    for (index, entry) in writers.iter().enumerate() {
        if index > 0 {
            out.push_str(" -> ");
        }
        let text = format!("{}={}", entry.block_id, entry.value.describe());
        if index == last_index {
            out.push_str(&style.green_bold(&format!("{text} (effective)")));
        } else {
            out.push_str(&text);
        }
    }
    out
}

fn describe_path_op(op: &PathOp) -> String {
    match op {
        PathOp::Prepend(path) => format!("prepend {path:?}"),
        PathOp::Append(path) => format!("append {path:?}"),
        PathOp::MoveFront(path) => format!("move_front {path:?}"),
        PathOp::MoveBack(path) => format!("move_back {path:?}"),
    }
}

struct Style {
    color: bool,
}

impl Style {
    fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }

    fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }

    fn cyan(&self, text: &str) -> String {
        self.paint("36", text)
    }

    fn yellow(&self, text: &str) -> String {
        self.paint("33", text)
    }

    fn green_bold(&self, text: &str) -> String {
        self.paint("1;32", text)
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::config::{BlockConfigToml, EnvValue, RawConfig, ShellOverridesToml};
    use crate::resolve::resolve_with_details;

    #[test]
    fn renders_explain_output_shape() {
        let mut base = BlockConfigToml::default();
        base.env.insert("EDITOR".into(), "vim".into());
        base.before.push("nvim".into());

        let mut nvim = BlockConfigToml::default();
        nvim.when.push("interactive".into());
        nvim.alias.insert("vim".into(), "nvim".into());
        nvim.env.insert("EDITOR".into(), "nvim".into());
        nvim.after.push("base".into());

        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("base".into(), base), ("nvim".into(), nvim)]),
        };

        let resolution = resolve_with_details(&raw, "fish").unwrap();
        let text = render_resolution(&resolution, RenderOptions::default());

        assert!(text.contains("conch explain fish"));
        assert!(text.contains("Block order"));
        assert!(text.contains("  1. base"));
        assert!(text.contains("  2. nvim"));
        assert!(text.contains("Blocks"));
        assert!(text.contains("base  unguarded"));
        assert!(text.contains("nvim  guarded"));
        assert!(text.contains("when        interactive"));
        assert!(text.contains("contributes env EDITOR=\"vim\""));
        assert!(text.contains("Env bindings"));
        assert!(text.contains("base=\"vim\" -> nvim=\"nvim\" (effective)"));
    }

    #[test]
    fn includes_source_line_counts_in_block_contributions() {
        let mut starship = BlockConfigToml::default();
        starship.when.push("interactive".into());
        starship.shell.insert(
            "fish".into(),
            ShellOverridesToml {
                source_lines: vec!["starship init fish | source".into()],
                ..Default::default()
            },
        );

        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("starship".into(), starship)]),
        };

        let resolution = resolve_with_details(&raw, "fish").unwrap();
        let text = render_resolution(&resolution, RenderOptions::default());

        assert!(text.contains("starship  guarded"));
        assert!(text.contains("contributes source 1 verbatim line"));
    }

    #[test]
    fn renders_typed_env_values_in_explain_output() {
        let mut block = BlockConfigToml::default();
        block.env.insert("ENABLED".into(), EnvValue::Bool(true));
        block
            .env
            .insert("RETRIES".into(), EnvValue::Integer("1".into()));
        block
            .env
            .insert("EDITOR".into(), EnvValue::String("nvim".into()));

        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution = resolve_with_details(&raw, "fish").unwrap();
        let text = render_resolution(&resolution, RenderOptions::default());

        assert!(text.contains("env ENABLED=true"));
        assert!(text.contains("env RETRIES=1"));
        assert!(text.contains("env EDITOR=\"nvim\""));
        assert!(text.contains("demo=true (effective)"));
        assert!(text.contains("demo=1 (effective)"));
        assert!(text.contains("demo=\"nvim\" (effective)"));
    }

    #[test]
    fn renders_ansi_styles_when_color_is_enabled() {
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("base".into(), BlockConfigToml::default())]),
        };
        let resolution = resolve_with_details(&raw, "fish").unwrap();
        let text = render_resolution(&resolution, RenderOptions { color: true });

        assert!(text.contains("\x1b[1mconch explain\x1b[0m"));
        assert!(text.contains("\x1b[36mfish\x1b[0m"));
    }
}
