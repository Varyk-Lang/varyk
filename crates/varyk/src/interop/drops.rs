//! Finding the types a `.rs` file gives a destructor (`impl Drop`), so a
//! `match` on a temporary of such an enum never moves a payload out
//! (rustc E0509). The scan walks the whole file (inline modules, function
//! bodies, `const _` blocks), and reports what it cannot pin down so the
//! resolver can mark every enum instead: marking too many only makes a
//! `match` look inside its value instead of taking it apart, which is
//! always sound. A file that renames or re-exports `Drop` (`use ... Drop
//! as X`, `pub use ... Drop`, `use std::ops::*`) counts as such: another
//! file can implement `Drop` under a name this scan never sees. So does a
//! macro, invoked or defined anywhere (an item, a statement, an argument
//! of `println!`), whose tokens spell `Drop`: its expansion could be an
//! `impl Drop` no scan can read. So does any other macro call, in any
//! position and at any depth, that [`Macros::risk`] cannot rule out: one
//! defined elsewhere (other than the few `std` macros it knows), one whose
//! definition in this file spells an item keyword or calls another macro,
//! or one whose own tokens call a macro it does not know.

use syn::visit::{self, Visit};
use syn::{
    File, ItemImpl, ItemMacro, ItemStruct, ItemType, ItemUnion, ItemUse, Macro, Type, UseRename,
    UseTree, Visibility,
};

use super::macros::{MacroRisk, Macros, spells_drop};
use super::{DropScan, ident_name};

/// Scans all of `file` for `impl Drop for X` (any trait path whose last
/// segment is `Drop`, or a name `Drop` was renamed to by `use ... as`).
pub(super) fn scan(file: &File, macros: &Macros) -> DropScan {
    let mut visitor = Visitor {
        macros,
        impls: Vec::new(),
        drop_renames: Vec::new(),
        aliases: Vec::new(),
        structs: Vec::new(),
        drop_shared: false,
        macro_reason: None,
    };
    visitor.visit_file(file);
    let mut out = DropScan {
        aliases: visitor.aliases,
        structs: visitor.structs,
        unknown: match visitor.macro_reason {
            Some(why) => Some(why),
            None if visitor.drop_shared => {
                Some("renames, re-exports, or glob-imports `Drop`".to_string())
            }
            None => None,
        },
        ..DropScan::default()
    };
    for (trait_name, target) in visitor.impls {
        if trait_name != "Drop" && !visitor.drop_renames.contains(&trait_name) {
            continue;
        }
        match target {
            Some(name) => out.targets.push(name),
            None => {
                out.unknown.get_or_insert_with(|| {
                    "implements `Drop` for a type Varyk cannot resolve".to_string()
                });
            }
        }
    }
    out
}

struct Visitor<'m, 'a> {
    /// The file's `macro_rules!` definitions, to judge a macro call.
    macros: &'m Macros<'a>,
    /// Every trait impl: the trait path's last segment, and the self
    /// type's last segment when it is a plain path.
    impls: Vec<(String, Option<String>)>,
    /// Names `Drop` itself was renamed to (`use std::ops::Drop as D;`).
    drop_renames: Vec<String>,
    /// Names a `type` alias or a `use ... as` rename introduces.
    aliases: Vec<String>,
    /// Every struct and union name declared anywhere in the file.
    structs: Vec<String>,
    /// `Drop` is renamed, re-exported, or glob-imported from `ops`, so
    /// another file may implement it under a name no scan can match.
    drop_shared: bool,
    /// Why the first macro that could write an `impl Drop` no scan can
    /// see could: one whose tokens spell `Drop` (a `macro_rules!` or a
    /// call, in any position), or a call, in any position, that
    /// [`Macros::risk`] cannot rule out. The end of a sentence starting
    /// with the file's name.
    macro_reason: Option<String>,
}

impl<'ast> Visit<'ast> for Visitor<'_, '_> {
    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        if let Some((path, _)) = &item.trait_ {
            if let Some(last) = path.segments.last() {
                let target = match item.self_ty.as_ref() {
                    Type::Path(ty) if ty.qself.is_none() => {
                        ty.path.segments.last().map(|seg| ident_name(&seg.ident))
                    }
                    _ => None,
                };
                self.impls.push((ident_name(&last.ident), target));
            }
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast ItemMacro) {
        match &item.ident {
            // A `macro_rules!` definition, named by the macro it defines.
            Some(defined) => {
                if spells_drop(&item.mac.tokens) {
                    let name = ident_name(defined);
                    self.macro_reason.get_or_insert_with(|| {
                        format!("has a macro, `{name}!`, whose text mentions `Drop`")
                    });
                }
            }
            None => self.judge(&item.mac, true),
        }
    }

    /// A call in a statement or an expression (or where an associated
    /// item goes); the item-position calls come to `visit_item_macro`.
    fn visit_macro(&mut self, mac: &'ast Macro) {
        self.judge(mac, false);
    }

    fn visit_item_type(&mut self, item: &'ast ItemType) {
        self.aliases.push(ident_name(&item.ident));
        visit::visit_item_type(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let public = !matches!(item.vis, Visibility::Inherited);
        if shares_drop(&item.tree, public, None) {
            self.drop_shared = true;
        }
        visit::visit_item_use(self, item);
    }

    fn visit_use_rename(&mut self, rename: &'ast UseRename) {
        let new = ident_name(&rename.rename);
        if ident_name(&rename.ident) == "Drop" {
            self.drop_renames.push(new.clone());
        }
        self.aliases.push(new);
        visit::visit_use_rename(self, rename);
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        self.structs.push(ident_name(&item.ident));
        visit::visit_item_struct(self, item);
    }

    fn visit_item_union(&mut self, item: &'ast ItemUnion) {
        self.structs.push(ident_name(&item.ident));
        visit::visit_item_union(self, item);
    }
}

impl Visitor<'_, '_> {
    /// Records why the call `mac` (`item`: in item position) could write
    /// an `impl Drop` no scan can see, unless an earlier macro already
    /// did.
    fn judge(&mut self, mac: &Macro, item: bool) {
        if self.macro_reason.is_some() {
            return;
        }
        let name = macro_name(mac);
        if mac.path.segments.iter().any(|seg| seg.ident == "Drop") || spells_drop(&mac.tokens) {
            self.macro_reason = Some(format!(
                "has a macro, `{name}!`, whose text mentions `Drop`"
            ));
            return;
        }
        self.macro_reason = self.macros.risk(mac, item).map(|risk| match risk {
            MacroRisk::Writes => {
                format!("calls `{name}!`, a macro that could write an `impl Drop` Varyk cannot see")
            }
            MacroRisk::Relays => format!(
                "calls `{name}!`, which calls a macro defined elsewhere that could write an \
                 `impl Drop` Varyk cannot see"
            ),
            MacroRisk::Elsewhere => format!(
                "calls `{name}!`, a macro defined elsewhere that could write an `impl Drop` \
                 Varyk cannot see"
            ),
        });
    }
}

/// Whether a `use` tree renames `Drop`, re-exports it (`public`), or
/// globs a module named `ops` (`last` is the segment before the tree).
fn shares_drop(tree: &UseTree, public: bool, last: Option<String>) -> bool {
    match tree {
        UseTree::Path(path) => shares_drop(&path.tree, public, Some(ident_name(&path.ident))),
        UseTree::Name(name) => public && ident_name(&name.ident) == "Drop",
        UseTree::Rename(rename) => ident_name(&rename.ident) == "Drop",
        UseTree::Glob(_) => last.as_deref() == Some("ops"),
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|item| shares_drop(item, public, last.clone())),
    }
}

/// The last segment of a macro call's path: `make` for `dep::make!`.
fn macro_name(mac: &Macro) -> String {
    mac.path
        .segments
        .last()
        .map_or_else(String::new, |seg| ident_name(&seg.ident))
}
