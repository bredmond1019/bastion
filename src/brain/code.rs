//! Pure extraction of Rust symbols and references via tree-sitter.
//!
//! This module is the extraction layer for the code-as-graph surface (Block C).
//! All public functions are deterministic over `(source: &str, path: &Path)` —
//! no I/O, no LLM, no network.
//!
//! # Coverage
//! Extraction scope is the **Rust language only**, using the `tree-sitter-rust`
//! grammar. Symbol kinds: `Fn`, `Struct`, `Enum`, `Trait`, `Mod`, `Impl`.
//! Reference kinds: direct function calls, method calls, and `use` import paths.
//! Other languages in the scan root are silently skipped by the file-walk layer.

use std::path::{Path, PathBuf};

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

// ── Language singleton ────────────────────────────────────────────────────────

fn rust_language() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// The category of a symbol definition extracted from Rust source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// A `fn` item — top-level, method inside `impl`, or nested.
    Fn,
    /// A `struct` item.
    Struct,
    /// An `enum` item.
    Enum,
    /// A `trait` item.
    Trait,
    /// A `mod` item (inline or file module declaration).
    Mod,
    /// An `impl` block — keyed by the implementing type name.
    Impl,
}

/// A symbol definition found in Rust source.
#[derive(Debug, Clone)]
pub struct CodeSymbol {
    /// Symbol name as written in source (identifier or type identifier).
    pub name: String,
    /// The kind of definition.
    pub kind: SymbolKind,
    /// Source file path.
    pub path: PathBuf,
    /// 1-indexed source line of the definition keyword / name.
    pub line: usize,
}

/// A reference to a symbol — a call site or `use` import.
///
/// References to extern / std symbols are included; the graph-build layer
/// (`code_graph.rs`) drops refs that do not resolve to a known symbol.
#[derive(Debug, Clone)]
pub struct CodeRef {
    /// The referenced name (last path segment for `use`, callee for calls).
    pub name: String,
    /// Source file that contains this reference.
    pub path: PathBuf,
    /// 1-indexed source line.
    pub line: usize,
}

// ── Pure extraction ───────────────────────────────────────────────────────────

/// Extract all symbol definitions from `source` using tree-sitter.
///
/// Returns symbols for every `fn`, `struct`, `enum`, `trait`, `mod`, and `impl`
/// item found anywhere in the parse tree, including methods nested inside `impl`
/// blocks and `fn` items nested inside other `fn` items.
///
/// Tree-sitter recovers from malformed / partial source; extraction returns
/// whatever it can without panicking.
pub fn extract_symbols(source: &str, path: &Path) -> Vec<CodeSymbol> {
    let lang = rust_language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let bytes = source.as_bytes();
    let mut symbols = Vec::new();

    // Each tuple: (tree-sitter query pattern, SymbolKind to assign matched captures).
    // All queries target the `@name` capture — the identifier or type-identifier node
    // that holds the symbol's name text.
    let queries: &[(&str, SymbolKind)] = &[
        (
            r#"(function_item name: (identifier) @name)"#,
            SymbolKind::Fn,
        ),
        (
            r#"(struct_item name: (type_identifier) @name)"#,
            SymbolKind::Struct,
        ),
        (
            r#"(enum_item name: (type_identifier) @name)"#,
            SymbolKind::Enum,
        ),
        (
            r#"(trait_item name: (type_identifier) @name)"#,
            SymbolKind::Trait,
        ),
        (r#"(mod_item name: (identifier) @name)"#, SymbolKind::Mod),
        (
            r#"(impl_item type: (type_identifier) @name)"#,
            SymbolKind::Impl,
        ),
        // Generic impl blocks: `impl<T> Container<T>` — type field is generic_type
        (
            r#"(impl_item type: (generic_type type: (type_identifier) @name))"#,
            SymbolKind::Impl,
        ),
    ];

    for (query_src, kind) in queries {
        let query = match Query::new(&lang, query_src) {
            Ok(q) => q,
            Err(_) => continue,
        };
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), bytes);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let name = match cap.node.utf8_text(bytes) {
                    Ok(t) => t.to_string(),
                    Err(_) => continue,
                };
                let line = cap.node.start_position().row + 1; // row is 0-indexed
                symbols.push(CodeSymbol {
                    name,
                    kind: kind.clone(),
                    path: path.to_path_buf(),
                    line,
                });
            }
        }
    }

    symbols
}

/// Extract all references (call sites and `use` imports) from `source` via tree-sitter.
///
/// A `CodeRef` is emitted for each:
/// - Direct function call: `alpha()` → name `alpha`
/// - Method call: `widget.render()` → name `render`
/// - `use` import with scoped path: `use lib::alpha` → name `alpha`
/// - `use` import with bare identifier: `use alpha` → name `alpha`
pub fn extract_refs(source: &str, path: &Path) -> Vec<CodeRef> {
    let lang = rust_language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let bytes = source.as_bytes();
    let mut refs = Vec::new();

    // Each query targets the `@name` capture — the leaf identifier of the reference.
    let query_patterns: &[&str] = &[
        // Direct function calls: alpha()
        r#"(call_expression function: (identifier) @name)"#,
        // Method calls: widget.render()
        r#"(call_expression function: (field_expression field: (field_identifier) @name))"#,
        // use imports with scoped path — last segment: use lib::alpha
        r#"(use_declaration argument: (scoped_identifier name: (identifier) @name))"#,
        // use imports with bare identifier: use alpha
        r#"(use_declaration argument: (identifier) @name)"#,
        // turbofish calls: foo::<u32>() — function field is generic_function
        r#"(call_expression function: (generic_function function: (identifier) @name))"#,
    ];

    for query_src in query_patterns {
        let query = match Query::new(&lang, query_src) {
            Ok(q) => q,
            Err(_) => continue,
        };
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), bytes);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let name = match cap.node.utf8_text(bytes) {
                    Ok(t) => t.to_string(),
                    Err(_) => continue,
                };
                let line = cap.node.start_position().row + 1;
                refs.push(CodeRef {
                    name,
                    path: path.to_path_buf(),
                    line,
                });
            }
        }
    }

    refs
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // Fixture topology (used to assert exact counts and line numbers):
    //
    // lib.rs.fixture:
    //   L1  pub fn alpha() {}                → alpha (Fn)
    //   L3  pub fn beta() {}                 → beta (Fn)
    //   L5  pub struct Widget { ... }        → Widget (Struct)
    //   L9  impl Widget { ... }              → Widget (Impl)
    //   L10     pub fn render(&self) {}      → render (Fn)
    //   Total: 5 symbols, 0 refs
    //
    // consumer.rs.fixture:
    //   L1  use lib::alpha;                  → ref alpha (use)
    //   L2  use lib::beta;                   → ref beta (use)
    //   L3  use lib::Widget;                 → ref Widget (use)
    //   L5  pub fn main_consumer() { ... }   → main_consumer (Fn)
    //   L6      alpha();                     → ref alpha (call)
    //   L7      beta();                      → ref beta (call)
    //   L10     w2.render();                 → ref render (method call)
    //   Total: 1 symbol, 6 refs
    //
    // util.rs.fixture:
    //   L1  pub fn isolated_helper() {}      → isolated_helper (Fn)
    //   Total: 1 symbol, 0 refs
    const LIB_FIXTURE: &str = include_str!("fixtures/code/lib.rs.fixture");
    const CONSUMER_FIXTURE: &str = include_str!("fixtures/code/consumer.rs.fixture");
    const UTIL_FIXTURE: &str = include_str!("fixtures/code/util.rs.fixture");

    // ── extract_symbols: lib.rs.fixture ──────────────────────────────────────

    #[test]
    fn symbols_lib_alpha_fn_line1() {
        let syms = extract_symbols(LIB_FIXTURE, Path::new("lib.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "alpha" && s.kind == SymbolKind::Fn);
        assert!(
            found.is_some(),
            "expected fn alpha; got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 1, "alpha must be at line 1");
    }

    #[test]
    fn symbols_lib_beta_fn_line3() {
        let syms = extract_symbols(LIB_FIXTURE, Path::new("lib.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "beta" && s.kind == SymbolKind::Fn);
        assert!(
            found.is_some(),
            "expected fn beta; got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 3, "beta must be at line 3");
    }

    #[test]
    fn symbols_lib_widget_struct_line5() {
        let syms = extract_symbols(LIB_FIXTURE, Path::new("lib.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "Widget" && s.kind == SymbolKind::Struct);
        assert!(
            found.is_some(),
            "expected struct Widget; got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 5, "Widget struct must be at line 5");
    }

    #[test]
    fn symbols_lib_widget_impl_line9() {
        let syms = extract_symbols(LIB_FIXTURE, Path::new("lib.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "Widget" && s.kind == SymbolKind::Impl);
        assert!(
            found.is_some(),
            "expected impl Widget; got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 9, "impl Widget must be at line 9");
    }

    #[test]
    fn symbols_lib_render_fn_in_impl_line10() {
        let syms = extract_symbols(LIB_FIXTURE, Path::new("lib.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "render" && s.kind == SymbolKind::Fn);
        assert!(
            found.is_some(),
            "expected fn render (method in impl); got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 10, "render must be at line 10");
    }

    #[test]
    fn symbols_lib_total_count_is_5() {
        // alpha(Fn) + beta(Fn) + Widget(Struct) + Widget(Impl) + render(Fn) = 5
        let syms = extract_symbols(LIB_FIXTURE, Path::new("lib.rs"));
        assert_eq!(
            syms.len(),
            5,
            "lib fixture must yield exactly 5 symbols; got: {:?}",
            debug_syms(&syms)
        );
    }

    // ── extract_symbols: consumer.rs.fixture ─────────────────────────────────

    #[test]
    fn symbols_consumer_main_consumer_fn_line5() {
        let syms = extract_symbols(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "main_consumer" && s.kind == SymbolKind::Fn);
        assert!(
            found.is_some(),
            "expected fn main_consumer; got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 5, "main_consumer must be at line 5");
    }

    #[test]
    fn symbols_consumer_total_count_is_1() {
        let syms = extract_symbols(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        assert_eq!(
            syms.len(),
            1,
            "consumer fixture must yield exactly 1 symbol; got: {:?}",
            debug_syms(&syms)
        );
    }

    // ── extract_symbols: util.rs.fixture ─────────────────────────────────────

    #[test]
    fn symbols_util_isolated_helper_fn_line1() {
        let syms = extract_symbols(UTIL_FIXTURE, Path::new("util.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "isolated_helper" && s.kind == SymbolKind::Fn);
        assert!(
            found.is_some(),
            "expected fn isolated_helper; got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 1, "isolated_helper must be at line 1");
    }

    // ── extract_refs: consumer.rs.fixture ────────────────────────────────────

    #[test]
    fn refs_consumer_use_alpha_line1() {
        let refs = extract_refs(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        let found = refs.iter().any(|r| r.name == "alpha" && r.line == 1);
        assert!(
            found,
            "expected use ref 'alpha' at line 1; refs: {:?}",
            debug_refs(&refs)
        );
    }

    #[test]
    fn refs_consumer_use_beta_line2() {
        let refs = extract_refs(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        let found = refs.iter().any(|r| r.name == "beta" && r.line == 2);
        assert!(
            found,
            "expected use ref 'beta' at line 2; refs: {:?}",
            debug_refs(&refs)
        );
    }

    #[test]
    fn refs_consumer_use_widget_line3() {
        let refs = extract_refs(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        let found = refs.iter().any(|r| r.name == "Widget" && r.line == 3);
        assert!(
            found,
            "expected use ref 'Widget' at line 3; refs: {:?}",
            debug_refs(&refs)
        );
    }

    #[test]
    fn refs_consumer_call_alpha_line6() {
        let refs = extract_refs(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        let found = refs.iter().any(|r| r.name == "alpha" && r.line == 6);
        assert!(
            found,
            "expected call ref 'alpha' at line 6; refs: {:?}",
            debug_refs(&refs)
        );
    }

    #[test]
    fn refs_consumer_call_beta_line7() {
        let refs = extract_refs(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        let found = refs.iter().any(|r| r.name == "beta" && r.line == 7);
        assert!(
            found,
            "expected call ref 'beta' at line 7; refs: {:?}",
            debug_refs(&refs)
        );
    }

    #[test]
    fn refs_consumer_method_render() {
        let refs = extract_refs(CONSUMER_FIXTURE, Path::new("consumer.rs"));
        let found = refs.iter().any(|r| r.name == "render");
        assert!(
            found,
            "expected method-call ref 'render'; refs: {:?}",
            debug_refs(&refs)
        );
    }

    // ── util fixture has no refs ──────────────────────────────────────────────

    #[test]
    fn refs_util_is_empty() {
        let refs = extract_refs(UTIL_FIXTURE, Path::new("util.rs"));
        assert!(
            refs.is_empty(),
            "util fixture must have no refs; got: {:?}",
            debug_refs(&refs)
        );
    }

    // ── boundary: malformed / partial source ─────────────────────────────────

    #[test]
    fn extract_symbols_malformed_no_panic() {
        // tree-sitter recovers; extraction must not panic
        let malformed = "fn broken( { struct X {}";
        let syms = extract_symbols(malformed, Path::new("broken.rs"));
        // Partial parse may or may not yield symbols — the contract is: no panic
        let _ = syms;
    }

    #[test]
    fn extract_refs_malformed_no_panic() {
        let malformed = "fn foo( { use ::; alpha()";
        let refs = extract_refs(malformed, Path::new("broken.rs"));
        let _ = refs;
    }

    #[test]
    fn extract_symbols_empty_source_returns_empty() {
        let syms = extract_symbols("", Path::new("empty.rs"));
        assert!(syms.is_empty(), "empty source must yield no symbols");
    }

    #[test]
    fn extract_refs_empty_source_returns_empty() {
        let refs = extract_refs("", Path::new("empty.rs"));
        assert!(refs.is_empty(), "empty source must yield no refs");
    }

    // ── boundary: isolated symbol has no refs ────────────────────────────────

    #[test]
    fn isolated_symbol_no_refs_in_util() {
        // util.rs.fixture contains `isolated_helper` — nothing calls it.
        // Confirm the fixture itself contains no refs.
        let refs = extract_refs(UTIL_FIXTURE, Path::new("util.rs"));
        assert!(
            refs.is_empty(),
            "isolated_helper has no call sites; refs should be empty"
        );
    }

    // ── boundary: enum and trait extraction ──────────────────────────────────

    #[test]
    fn extract_symbols_enum() {
        let src = "pub enum Color { Red, Green, Blue }";
        let syms = extract_symbols(src, Path::new("color.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "Color" && s.kind == SymbolKind::Enum);
        assert!(found.is_some(), "expected enum Color");
        assert_eq!(found.unwrap().line, 1);
    }

    #[test]
    fn extract_symbols_trait() {
        let src = "pub trait Drawable { fn draw(&self); }";
        let syms = extract_symbols(src, Path::new("drawable.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "Drawable" && s.kind == SymbolKind::Trait);
        assert!(found.is_some(), "expected trait Drawable");
        assert_eq!(found.unwrap().line, 1);
    }

    #[test]
    fn extract_symbols_mod() {
        let src = "mod utils { pub fn helper() {} }";
        let syms = extract_symbols(src, Path::new("main.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "utils" && s.kind == SymbolKind::Mod);
        assert!(found.is_some(), "expected mod utils");
        assert_eq!(found.unwrap().line, 1);
    }

    #[test]
    fn extract_symbols_generic_impl() {
        let src = "impl<T> Container<T> { pub fn len(&self) -> usize { 0 } }";
        let syms = extract_symbols(src, Path::new("container.rs"));
        let found = syms
            .iter()
            .find(|s| s.name == "Container" && s.kind == SymbolKind::Impl);
        assert!(
            found.is_some(),
            "generic impl<T> Container<T> must yield Impl symbol; got: {:?}",
            debug_syms(&syms)
        );
        assert_eq!(found.unwrap().line, 1);
    }

    #[test]
    fn extract_refs_turbofish_call() {
        let src = r#"fn foo() { let v = parse::<u32>("42"); }"#;
        let refs = extract_refs(src, Path::new("foo.rs"));
        let found = refs.iter().any(|r| r.name == "parse");
        assert!(
            found,
            "turbofish call parse::<u32>() must yield a ref; refs: {:?}",
            debug_refs(&refs)
        );
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn debug_syms(syms: &[CodeSymbol]) -> Vec<(&str, &SymbolKind, usize)> {
        syms.iter()
            .map(|s| (s.name.as_str(), &s.kind, s.line))
            .collect()
    }

    fn debug_refs(refs: &[CodeRef]) -> Vec<(&str, usize)> {
        refs.iter().map(|r| (r.name.as_str(), r.line)).collect()
    }
}
