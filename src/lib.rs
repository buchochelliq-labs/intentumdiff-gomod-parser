//! go.mod parser plugin — full-parse mode on tree-sitter-gomod (issue #48). A go.mod is
//! a KEYED dependency manifest: review identity lives in module paths, so require/
//! replace/exclude specs are labeled by their module path and keep the version as a
//! semantic child — a version bump pairs as ONE MODIFICATION under a stable identity.

use intentumdiff_plugin_sdk::{
    cst::CstNode,
    ts_convert::{convert_semantic, node_to_cst},
    tree::SemanticNodeBuilder,
};

wit_bindgen::generate!({
    path: "wit/plugin.wit",
    world: "parser-plugin",
});

use crate::exports::intentdiff::plugin::parser::ExamplePair;
use crate::exports::intentdiff::plugin::parser::Guest;
use crate::exports::intentdiff::plugin::parser::LanguageInfoRecord;
use crate::exports::intentdiff::plugin::parser::ParserMode;

const LANGUAGE_ID: &str = "gomod";
const PLUGIN_METADATA: &str = include_str!("../plugin_metadata.info");

const DEFAULT_OLD: &str =
    "module example.com/app\n\ngo 1.22\n\nrequire (\n\tgithub.com/pkg/errors v0.9.1\n)\n";
const DEFAULT_NEW: &str =
    "module example.com/app\n\ngo 1.22\n\nrequire (\n\tgithub.com/pkg/errors v0.9.2\n)\n";

// Directives and specs carry review meaning; parens, keywords and comments are dropped.
const SEMANTIC_TYPES: &[&str] = &[
    "source_file",
    "module_directive",
    "go_directive",
    "toolchain_directive",
    "require_directive",
    "require_spec",
    "replace_directive",
    "replace_spec",
    "exclude_directive",
    "exclude_spec",
    "retract_directive",
    "retract_spec",
    "module_path",
    "go_version",
    "version",
    "file_path",
];

fn is_semantic(node_type: &str) -> bool {
    SEMANTIC_TYPES.contains(&node_type)
}

fn language_info_for(ids: Vec<String>) -> Vec<LanguageInfoRecord> {
    let metadata = intentumdiff_plugin_sdk::metadata::parse_plugin_metadata(PLUGIN_METADATA);
    ids.into_iter()
        .map(|language_id| {
            let info = metadata.language_or_default(&language_id);
            LanguageInfoRecord {
                language_id: info.language_id,
                language_name: info.language_name,
                language_short_name: info.language_short_name,
                monaco_language: info.monaco_language,
                default_filename: info.default_filename,
                language_file_extensions: info.language_file_extensions,
                author: metadata.author().to_string(),
                plugin_version: metadata.plugin_version().to_string(),
                last_updated: metadata.last_updated().to_string(),
            }
        })
        .collect()
}

fn basename(path: &str) -> &str {
    path.rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or(path)
}

fn detect_language_impl(filename: &str, _content: &str) -> String {
    if basename(filename).eq_ignore_ascii_case("go.mod") {
        LANGUAGE_ID.to_string()
    } else {
        String::new()
    }
}

/// First non-empty LEAF text under `node` (CstNode only carries text on leaves).
fn leaf_text(node: &CstNode) -> Option<String> {
    if node.is_leaf() {
        let text = node.text_or_empty().trim();
        if !text.is_empty() {
            return Some(text.chars().take(120).collect());
        }
        return None;
    }
    node.children.iter().find_map(leaf_text)
}

/// First descendant of `key_type`, read via its leaves.
fn key_text(node: &CstNode, key_type: &str) -> Option<String> {
    fn find_key(node: &CstNode, key_type: &str) -> Option<String> {
        if node.node_type == key_type {
            if let Some(text) = leaf_text(node) {
                return Some(text);
            }
        }
        for child in &node.children {
            if let Some(text) = find_key(child, key_type) {
                return Some(text);
            }
        }
        None
    }
    find_key(node, key_type)
}

fn label_for(node: &CstNode) -> String {
    if node.is_leaf() {
        return node.text_or_empty().trim().chars().take(120).collect();
    }
    match node.node_type.as_str() {
        // Specs and the module directive are identified by their module path.
        "module_directive" | "require_spec" | "replace_spec" | "exclude_spec" => {
            key_text(node, "module_path").unwrap_or_else(|| node.node_type.clone())
        }
        "go_directive" => key_text(node, "go_version").unwrap_or_else(|| "go".to_string()),
        "retract_spec" => key_text(node, "version").unwrap_or_else(|| node.node_type.clone()),
        "module_path" | "version" | "go_version" | "file_path" => {
            leaf_text(node).unwrap_or_else(|| node.node_type.clone())
        }
        _ => node.node_type.clone(),
    }
}

fn parse_source(source: &str) -> Result<CstNode, String> {
    let mut parser = tree_sitter::Parser::new();
    let lang = tree_sitter_gomod::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|_| "Failed to load gomod grammar".to_string())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter failed to parse go.mod".to_string())?;
    Ok(node_to_cst(tree.root_node(), source.as_bytes()))
}

fn process_impl(source: &str) -> String {
    let cst = match parse_source(source) {
        Ok(cst) => cst,
        Err(err) => return format!(r#"{{"error":"{}"}}"#, err),
    };
    let mut memo = std::collections::HashMap::new();
    let node = convert_semantic(&cst, "0", &mut memo, &is_semantic, &label_for).unwrap_or_else(|| {
        SemanticNodeBuilder::new("0", "source_file", LANGUAGE_ID, 0, 0, 0, 0, "0").build()
    });
    match serde_json::to_string(&node) {
        Ok(serialized) => serialized,
        Err(err) => format!(r#"{{"error":"Serialisation error: {}"}}"#, err),
    }
}

struct GomodParser;

impl Guest for GomodParser {
    fn get_parser_mode() -> ParserMode {
        ParserMode::FullParse
    }

    fn grammar_id() -> String {
        LANGUAGE_ID.to_string()
    }

    fn detect_language(filename: String, content: String) -> String {
        detect_language_impl(&filename, &content)
    }

    fn preprocess_source(source: String) -> String {
        source
    }

    fn example(_language: String) -> ExamplePair {
        ExamplePair {
            old: DEFAULT_OLD.to_string(),
            new: DEFAULT_NEW.to_string(),
        }
    }

    fn process(input: String, _language: String, _filename: String) -> String {
        process_impl(&input)
    }

    fn trivia_node_types() -> Vec<String> {
        vec![]
    }

    fn language_ids() -> Vec<String> {
        vec![LANGUAGE_ID.to_string()]
    }

    fn language_info() -> Vec<LanguageInfoRecord> {
        language_info_for(Self::language_ids())
    }

    fn priority() -> i32 {
        5
    }
}

export!(GomodParser);

#[cfg(test)]
mod tests {
    use super::*;
    use intentumdiff_plugin_sdk::tree::SemanticNode;

    fn labels_by_type(node: &SemanticNode, node_type: &str, out: &mut Vec<String>) {
        if node.node_type == node_type {
            out.push(node.label.clone());
        }
        for child in &node.children {
            labels_by_type(child, node_type, out);
        }
    }

    #[test]
    fn parser_mode_is_full_parse() {
        assert_eq!(GomodParser::get_parser_mode(), ParserMode::FullParse);
    }

    #[test]
    fn detects_only_go_mod_files() {
        assert_eq!(detect_language_impl("go.mod", ""), LANGUAGE_ID);
        assert_eq!(detect_language_impl("services/api/go.mod", ""), LANGUAGE_ID);
        assert_eq!(detect_language_impl("go.sum", ""), "");
        assert_eq!(detect_language_impl("main.go", ""), "");
    }

    #[test]
    fn specs_are_labeled_by_module_path_with_version_children() {
        let parsed = process_impl(DEFAULT_NEW);
        intentumdiff_plugin_sdk::testing::assert_valid_json(&parsed, LANGUAGE_ID);
        let root: SemanticNode = serde_json::from_str(&parsed).unwrap();
        let mut specs = Vec::new();
        labels_by_type(&root, "require_spec", &mut specs);
        assert_eq!(specs, vec!["github.com/pkg/errors".to_string()], "specs: {specs:?}");
        let mut versions = Vec::new();
        labels_by_type(&root, "version", &mut versions);
        assert!(versions.contains(&"v0.9.2".to_string()), "versions: {versions:?}");
        let mut modules = Vec::new();
        labels_by_type(&root, "module_directive", &mut modules);
        assert_eq!(modules, vec!["example.com/app".to_string()], "modules: {modules:?}");
    }

    #[test]
    fn version_bump_changes_the_root_hash() {
        let old: SemanticNode = serde_json::from_str(&process_impl(DEFAULT_OLD)).unwrap();
        let new: SemanticNode = serde_json::from_str(&process_impl(DEFAULT_NEW)).unwrap();
        assert_ne!(old.structural_hash, new.structural_hash);
    }
}
