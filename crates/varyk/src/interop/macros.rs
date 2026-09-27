//! Whether a macro call of a `.rs` file could write items Varyk cannot
//! see: a type that shadows a name a signature spells (see
//! `collect_names`, which asks about top-level calls), or an `impl Drop`
//! (see `drops`, which asks about every call, in any position). One rule
//! serves both scans.
//!
//! A call resolves by its bare name to a `macro_rules!` of the same file,
//! at any depth (inline modules and function bodies too); when it is
//! found, its definition decides: an item keyword, the identifier `Drop`,
//! or a call of any macro in its text makes the call unknown. The call's
//! own tokens count as well, since a local macro can paste them as they
//! are: an item keyword, `Drop`, or a macro call among them makes it
//! unknown. A call that is not found (a path, `crate::make!`, a name a
//! `use` brings in, any call after a `#[macro_use] extern crate`, or a
//! macro of another file or crate) is unknown too, except a few macros of
//! the standard library: `thread_local!` in item position, and in a
//! statement or an expression the [`STD_MACROS`] (`println!`, `vec!`,
//! ...), by bare name or `std::`/`core::` path (a path without a
//! leading `::` only when no item or `use` of the file is named `std` or
//! `core`). Those are unknown only
//! when their tokens spell `Drop` (or, for `thread_local!`, an item
//! keyword) or hold a macro call that is unknown by this same rule, as an
//! expression: another of them is judged by its tokens, a local macro by
//! its definition, and any other macro is unknown.

use std::collections::{HashMap, HashSet};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};
use syn::{File, Item, ItemExternCrate, ItemMacro, Macro, Path, UseName, UseRename};

use super::ident_name;

/// Why a macro call could write items Varyk cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroRisk {
    /// Its tokens, or its definition in this file, spell an item keyword
    /// or `Drop`.
    Writes,
    /// Its definition in this file, or its own tokens, call another
    /// macro.
    Relays,
    /// It is not defined in this file.
    Elsewhere,
}

/// The `macro_rules!` definitions of one file and what decides whether a
/// bare-named call reaches them.
pub(super) struct Macros<'a> {
    /// Every `macro_rules!` body, by the macro's name.
    defs: HashMap<String, Vec<&'a TokenStream>>,
    /// Every name a `use` brings in, as written or renamed.
    used: HashSet<String>,
    /// A `#[macro_use] extern crate` brings in macros by bare name.
    macro_use: bool,
    /// An item or a `use` of this file, at any depth, is named `std` or
    /// `core`, so a path `std::name!` may not reach the standard library.
    shadows_std: bool,
}

impl<'a> Macros<'a> {
    pub(super) fn new(file: &'a File) -> Self {
        let mut macros = Macros {
            defs: HashMap::new(),
            used: HashSet::new(),
            macro_use: false,
            shadows_std: false,
        };
        macros.visit_file(file);
        macros.shadows_std |= ["std", "core"]
            .iter()
            .any(|krate| macros.used.contains(*krate));
        macros
    }

    /// Why the call `mac` could write items Varyk cannot see; `None` when
    /// it cannot. `item` says whether it stands where an item goes (at the
    /// top of a file or module); otherwise it is a statement or an
    /// expression.
    pub(super) fn risk(&self, mac: &Macro, item: bool) -> Option<MacroRisk> {
        self.judge(&mac.path, &mac.tokens, item)
    }

    /// [`Macros::risk`] of a call of `path` with `tokens`.
    fn judge(&self, path: &Path, tokens: &TokenStream, item: bool) -> Option<MacroRisk> {
        if let Some(defs) = self.local(path) {
            return if writes_items(tokens) || defs.iter().any(|body| writes_items(body)) {
                Some(MacroRisk::Writes)
            } else if calls_macro(tokens) || defs.iter().any(|body| calls_macro(body)) {
                Some(MacroRisk::Relays)
            } else {
                None
            };
        }
        let harmless = match self.std_name(path) {
            Some(name) if item => name == "thread_local",
            Some(name) => STD_MACROS.contains(&name.as_str()),
            None => false,
        };
        if !harmless {
            return Some(if writes_items(tokens) {
                MacroRisk::Writes
            } else {
                MacroRisk::Elsewhere
            });
        }
        let writes = if item {
            writes_items(tokens)
        } else {
            spells_drop(tokens)
        };
        if writes {
            Some(MacroRisk::Writes)
        } else if self.calls_unknown(tokens) {
            Some(MacroRisk::Relays)
        } else {
            None
        }
    }

    /// The definitions of the `macro_rules!` of this file a call of
    /// `path` reaches, when it is a bare name nothing else can bring in.
    fn local(&self, path: &Path) -> Option<&Vec<&'a TokenStream>> {
        let ident = path.get_ident()?;
        if path.leading_colon.is_some() || self.macro_use {
            return None;
        }
        let name = ident_name(ident);
        if self.used.contains(&name) {
            return None;
        }
        self.defs.get(&name)
    }

    /// The name of the standard library macro `path` calls: a bare name
    /// no `use`, `#[macro_use]`, or `macro_rules!` of this file takes, or
    /// a path `::std::name` or `::core::name`, or the same without the
    /// leading `::` when no item or `use` of this file is named `std` or
    /// `core`.
    fn std_name(&self, path: &Path) -> Option<String> {
        let names: Vec<String> = path
            .segments
            .iter()
            .map(|seg| ident_name(&seg.ident))
            .collect();
        match names.as_slice() {
            [name] if path.leading_colon.is_none() => {
                (!self.macro_use && !self.used.contains(name) && !self.defs.contains_key(name))
                    .then(|| name.clone())
            }
            [krate, name]
                if (krate == "std" || krate == "core")
                    && (path.leading_colon.is_some() || !self.shadows_std) =>
            {
                Some(name.clone())
            }
            _ => None,
        }
    }

    /// Whether `tokens`, at any depth, hold a macro call that, judged in
    /// an expression, could write items Varyk cannot see.
    fn calls_unknown(&self, tokens: &TokenStream) -> bool {
        let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
        trees.iter().enumerate().any(|(index, tree)| match tree {
            TokenTree::Group(group) => self.calls_unknown(&group.stream()),
            TokenTree::Punct(bang) if bang.as_char() == '!' => {
                match (call_path(&trees[..index]), trees.get(index + 1)) {
                    (Some(path), Some(TokenTree::Group(group))) => {
                        self.judge(&path, &group.stream(), false).is_some()
                    }
                    _ => false,
                }
            }
            _ => false,
        })
    }
}

/// The macros of the standard library that write no item of their own,
/// harmless in a statement or an expression when their tokens are.
const STD_MACROS: [&str; 34] = [
    "println",
    "print",
    "eprintln",
    "eprint",
    "format",
    "vec",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "panic",
    "write",
    "writeln",
    "dbg",
    "matches",
    "todo",
    "unimplemented",
    "unreachable",
    "concat",
    "stringify",
    "env",
    "line",
    "file",
    "column",
    "thread_local",
    "cfg",
    "format_args",
    "include_str",
    "include_bytes",
    "option_env",
    "compile_error",
    "module_path",
];

/// The path of a macro call whose `!` follows `before`: the trailing
/// identifiers joined by `::`, with a leading `::` if there is one;
/// `None` when `before` does not end in an identifier.
fn call_path(before: &[TokenTree]) -> Option<Path> {
    let colon = |tree: &TokenTree| matches!(tree, TokenTree::Punct(p) if p.as_char() == ':');
    let mut start = before.len();
    while start >= 1 && matches!(before[start - 1], TokenTree::Ident(_)) {
        start -= 1;
        if start >= 2 && colon(&before[start - 2]) && colon(&before[start - 1]) {
            start -= 2;
        } else {
            break;
        }
    }
    if start == before.len() {
        return None;
    }
    syn::parse2(before[start..].iter().cloned().collect()).ok()
}

impl<'a> Visit<'a> for Macros<'a> {
    fn visit_item(&mut self, item: &'a Item) {
        let ident = match item {
            Item::Const(item) => Some(&item.ident),
            Item::Enum(item) => Some(&item.ident),
            Item::ExternCrate(item) => Some(item.rename.as_ref().map_or(&item.ident, |(_, r)| r)),
            Item::Fn(item) => Some(&item.sig.ident),
            Item::Macro(item) => item.ident.as_ref(),
            Item::Mod(item) => Some(&item.ident),
            Item::Static(item) => Some(&item.ident),
            Item::Struct(item) => Some(&item.ident),
            Item::Trait(item) => Some(&item.ident),
            Item::TraitAlias(item) => Some(&item.ident),
            Item::Type(item) => Some(&item.ident),
            Item::Union(item) => Some(&item.ident),
            _ => None,
        };
        if ident.is_some_and(|ident| matches!(ident_name(ident).as_str(), "std" | "core")) {
            self.shadows_std = true;
        }
        visit::visit_item(self, item);
    }

    fn visit_item_macro(&mut self, item: &'a ItemMacro) {
        if let (Some(ident), true) = (&item.ident, item.mac.path.is_ident("macro_rules")) {
            self.defs
                .entry(ident_name(ident))
                .or_default()
                .push(&item.mac.tokens);
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_item_extern_crate(&mut self, item: &'a ItemExternCrate) {
        if item
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("macro_use"))
        {
            self.macro_use = true;
        }
        visit::visit_item_extern_crate(self, item);
    }

    fn visit_use_name(&mut self, name: &'a UseName) {
        self.used.insert(ident_name(&name.ident));
        visit::visit_use_name(self, name);
    }

    fn visit_use_rename(&mut self, rename: &'a UseRename) {
        self.used.insert(ident_name(&rename.rename));
        visit::visit_use_rename(self, rename);
    }
}

/// Whether `tokens`, at any depth, hold a keyword that starts an item
/// able to define or bring in a type name or an `impl`, or the name
/// `Drop`.
fn writes_items(tokens: &TokenStream) -> bool {
    tokens.clone().into_iter().any(|tree| match tree {
        TokenTree::Group(group) => writes_items(&group.stream()),
        TokenTree::Ident(ident) => matches!(
            ident_name(&ident).as_str(),
            "struct" | "enum" | "union" | "type" | "use" | "trait" | "fn" | "mod" | "impl" | "Drop"
        ),
        _ => false,
    })
}

/// Whether `tokens`, at any depth, hold a macro call: a name, `!`, and a
/// delimited group.
fn calls_macro(tokens: &TokenStream) -> bool {
    let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    trees.iter().enumerate().any(|(index, tree)| match tree {
        TokenTree::Group(group) => calls_macro(&group.stream()),
        TokenTree::Ident(_) => matches!(
            (trees.get(index + 1), trees.get(index + 2)),
            (Some(TokenTree::Punct(bang)), Some(TokenTree::Group(_))) if bang.as_char() == '!'
        ),
        _ => false,
    })
}

/// Whether `tokens`, at any depth, hold the identifier `Drop` (raw or not).
pub(super) fn spells_drop(tokens: &TokenStream) -> bool {
    tokens.clone().into_iter().any(|tree| match tree {
        TokenTree::Group(group) => spells_drop(&group.stream()),
        TokenTree::Ident(ident) => ident_name(&ident) == "Drop",
        _ => false,
    })
}
