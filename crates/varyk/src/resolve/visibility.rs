//! V0105 (spec 3.2): a module, or an item, without `pub` is visible in
//! the module that declares it and that module's descendants; a lookup
//! must see the item and every module on the way to it. A `pub` item's
//! signature or fields must not name a type some of its users cannot.

use varyk_syntax::{FixIt, Span};

use crate::diagnostics::{Diagnostic, codes};
use crate::types::Ty;

use super::{FnId, LookupError, ModuleId, StructId, Symbols, UserType};

/// Whether an item declared in `decl_module` is visible from `from`
/// (spec 3.2): it is `pub`, or `decl_module` is `from` or one of its
/// ancestors. Modules on the way to it are [`path_visible`]'s concern.
pub(crate) fn is_visible(
    symbols: &Symbols,
    from: ModuleId,
    decl_module: ModuleId,
    is_pub: bool,
) -> bool {
    is_pub || symbols.ancestors(from).any(|module| module == decl_module)
}

/// Whether every module from the crate root down to `target` is visible
/// from `from`, each by [`is_visible`] as an item of its parent; the
/// highest one that is not otherwise. The path as written does not
/// matter: the modules above where it starts are ancestors of `from`, and
/// those are always visible.
pub(crate) fn path_visible(
    symbols: &Symbols,
    from: ModuleId,
    target: ModuleId,
) -> Result<(), ModuleId> {
    let chain: Vec<ModuleId> = symbols.ancestors(target).collect();
    for &module in chain.iter().rev() {
        let scope = &symbols.scopes[module.0 as usize];
        let Some(parent) = scope.parent else {
            continue;
        };
        if !is_visible(symbols, from, parent, scope.is_pub) {
            return Err(module);
        }
    }
    Ok(())
}

/// The V0105 visibility check shared by the lookups, and the only place
/// one is decided: `None` when the item, declared in `item_module`, is
/// visible from `from`; otherwise the [`LookupError`] the lookup returns.
/// `private` is `None` for a `pub` item, else its declaration and the
/// keyword that starts it, where a `pub ` fix-it inserts.
pub(super) fn check_visible(
    symbols: &Symbols,
    from: ModuleId,
    item_module: ModuleId,
    private: Option<(Span, &'static str)>,
) -> Option<LookupError> {
    if let Err(module) = path_visible(symbols, from, item_module) {
        return Some(LookupError::PrivateModule { module });
    }
    match private {
        Some((decl, keyword)) if !is_visible(symbols, from, item_module, false) => {
            Some(LookupError::NotVisible { decl, keyword })
        }
        _ => None,
    }
}

impl Symbols {
    /// The module whose subtree can name an item declared in `module`
    /// (spec 3.2): `module` itself for an item without `pub`; for a `pub`
    /// one, the parent of the lowest module on its chain that is not
    /// `pub`, or the crate root when every module up to it is.
    pub(super) fn reach(&self, module: ModuleId, is_pub: bool) -> ModuleId {
        if !is_pub {
            return module;
        }
        let mut current = module;
        loop {
            let scope = &self.scopes[current.0 as usize];
            match scope.parent {
                Some(parent) if scope.is_pub => current = parent,
                Some(parent) => return parent,
                None => return current,
            }
        }
    }

    /// The reach of function `id`: for a method, the narrower of its own
    /// and its type's, since it can be used only where both can.
    pub(super) fn fn_reach(&self, id: FnId) -> ModuleId {
        let sig = &self.fns[id.0 as usize];
        let own = self.reach(sig.module, sig.is_pub);
        let owner = match sig.owner {
            Some(UserType::Struct(id)) => {
                let def = &self.structs[id.0 as usize];
                self.reach(def.module, def.is_pub)
            }
            Some(UserType::Enum(id)) => {
                let def = &self.enums[id.0 as usize];
                self.reach(def.module, def.is_pub)
            }
            None => own,
        };
        // Both are ancestors-or-self of the function's module, so one
        // contains the other.
        if self.contains(own, owner) {
            owner
        } else {
            own
        }
    }

    /// Whether `outer` is `inner` or one of its ancestors: every module
    /// that can see into `inner`'s subtree is in `outer`'s.
    fn contains(&self, outer: ModuleId, inner: ModuleId) -> bool {
        self.ancestors(inner).any(|module| module == outer)
    }

    /// V0105 when the resolved type `ty`, written at `span` in a `pub`
    /// item called `item` whose reach is `item_reach`, names a struct or
    /// enum, itself or inside `Option`, `Result`, or `Vec`, whose own
    /// reach is narrower: some user of the item could not name the type,
    /// and the generated Rust names it in full. The message asks for
    /// `pub` on the type when it has none, else on the module that stops
    /// its reach.
    pub(super) fn private_in_public(
        &self,
        ty: &Ty,
        span: Span,
        item: &str,
        item_reach: ModuleId,
    ) -> Option<Diagnostic> {
        let (name, module, is_pub, decl, keyword) = match ty {
            Ty::Option(inner) | Ty::Vec(inner) | Ty::Shared(inner) => {
                return self.private_in_public(inner, span, item, item_reach);
            }
            Ty::Result(ok, err) | Ty::HashMap(ok, err) => {
                return self
                    .private_in_public(ok, span, item, item_reach)
                    .or_else(|| self.private_in_public(err, span, item, item_reach));
            }
            Ty::Struct(id) => {
                let def = &self.structs[id.0 as usize];
                (&def.name, def.module, def.is_pub, def.span, "struct")
            }
            Ty::Enum(id) => {
                let def = &self.enums[id.0 as usize];
                (&def.name, def.module, def.is_pub, def.span, "enum")
            }
            _ => return None,
        };
        if self.contains(self.reach(module, is_pub), item_reach) {
            return None;
        }
        if !is_pub {
            let diagnostic = Diagnostic::new(
                codes::V0105,
                span,
                format!(
                    "`{name}` is used by a public item but is not public; add `pub` to the {keyword}"
                ),
            );
            return Some(declared_without_pub(diagnostic, decl, keyword));
        }
        let reach = self.reach(module, true);
        // The module that stops the type's reach: the one on its chain
        // declared in `reach`, without `pub`.
        let blocker = self
            .ancestors(module)
            .find(|&m| self.scopes[m.0 as usize].parent == Some(reach))
            .expect("a reach narrower than the root stops at a module on the chain");
        let (short, decl) = self.mod_name_and_decl(blocker);
        let owner = &self.scopes[reach.0 as usize].name;
        let diagnostic = Diagnostic::new(
            codes::V0105,
            span,
            format!(
                "`{name}` is in module `{short}`, which only `{owner}` can see, but `{item}` can \
                 be used from outside `{owner}`; write `pub mod {short};` (or make `{item}` private)"
            ),
        )
        .with_note(
            "in Rust terms, a public function or field names a type less visible than itself \
             (rustc's `private_interfaces` lint); Varyk makes it an error because the Rust it \
             generates names the type by its full path",
        );
        Some(declared_without_pub(diagnostic, decl, "mod"))
    }

    /// For an item imported from a `.rs` module whose reach is
    /// `item_reach` (M3 spec 4.1-4.3): when `ty` names a struct or enum,
    /// itself or inside `Option`, `Result`, or `Vec`, whose reach is
    /// narrower, a sentence naming the type, the module that stops its
    /// reach, and the `pub mod` that fixes it. The generated Rust names
    /// the type by its full path wherever the item is used, so such an
    /// item can be used only inside the module the type is seen from,
    /// which comes with the sentence.
    pub(super) fn hidden_type(&self, ty: &Ty, item_reach: ModuleId) -> Option<(String, ModuleId)> {
        let (name, module, is_pub) = match ty {
            Ty::Option(inner) | Ty::Vec(inner) => return self.hidden_type(inner, item_reach),
            Ty::Result(ok, err) | Ty::HashMap(ok, err) => {
                return self
                    .hidden_type(ok, item_reach)
                    .or_else(|| self.hidden_type(err, item_reach));
            }
            Ty::Struct(id) => {
                let def = &self.structs[id.0 as usize];
                (&def.name, def.module, def.is_pub)
            }
            Ty::Enum(id) => {
                let def = &self.enums[id.0 as usize];
                (&def.name, def.module, def.is_pub)
            }
            _ => return None,
        };
        let reach = self.reach(module, is_pub);
        if self.contains(reach, item_reach) {
            return None;
        }
        let owner = &self.scopes[reach.0 as usize].name;
        let Some(blocker) = self
            .ancestors(module)
            .find(|&m| self.scopes[m.0 as usize].parent == Some(reach))
        else {
            return Some((
                format!("`{name}` is not `pub`, so only `{owner}` can use it"),
                reach,
            ));
        };
        let (short, _) = self.mod_name_and_decl(blocker);
        let path = &self.scopes[module.0 as usize].name;
        Some((
            format!(
                "`{name}` is in module `{path}`, which only `{owner}` can see, so this can be \
                 used only inside `{owner}`; to use it here, write `pub mod {short};` in \
                 `{owner}`"
            ),
            reach,
        ))
    }

    /// Whether an imported item usable only inside `within` (see
    /// [`Self::hidden_type`]), or anywhere when `None`, can be used from
    /// module `from`.
    pub(crate) fn usable_from(&self, within: Option<ModuleId>, from: ModuleId) -> bool {
        within.is_none_or(|within| self.contains(within, from))
    }

    /// V0105 at `span` for `what` (`function`, `type`, ...) `path`, whose
    /// module path passes through `module`, which is not visible from
    /// here: the fix-it adds `pub` to its `mod` declaration.
    pub(crate) fn private_module(
        &self,
        module: ModuleId,
        what: &str,
        path: &str,
        span: Span,
    ) -> Diagnostic {
        let (short, decl) = self.mod_name_and_decl(module);
        let parent = self.scopes[module.0 as usize]
            .parent
            .expect("the root is always visible");
        let owner = &self.scopes[parent.0 as usize].name;
        let diagnostic = Diagnostic::new(
            codes::V0105,
            span,
            format!(
                "module `{short}` is private to `{owner}`, so {what} `{path}` cannot be used \
                 here; write `pub mod {short};`"
            ),
        )
        .with_note(
            "a module declared without `pub` can be used only by the module that declares it \
             and the modules inside that one",
        );
        declared_without_pub(diagnostic, decl, "mod")
    }

    /// The name `module` is declared under and its `mod` declaration.
    fn mod_name_and_decl(&self, module: ModuleId) -> (&str, Span) {
        let scope = &self.scopes[module.0 as usize];
        let short = scope.name.rsplit("::").next().unwrap_or(&scope.name);
        let decl = scope.decl.expect("every module but the root has a `mod`");
        (short, decl)
    }

    /// V0105 at `span` for reading or assigning field `field` (its index
    /// into `struct_id`'s fields) from outside its declaring module (spec
    /// 3.4): a note offers both ways to fix it. The label and fix-it
    /// point at the field's own declaration, which may be in another
    /// file; a `pub ` fix-it inserts right before its name.
    pub(crate) fn private_field(
        &self,
        struct_id: StructId,
        field: usize,
        span: Span,
    ) -> Diagnostic {
        let def = &self.structs[struct_id.0 as usize];
        let decl = &def.fields[field];
        let module = &self.scopes[def.module.0 as usize].name;
        // A Rust field may be `pub(crate)`, where a `pub ` fix-it would not
        // make valid Rust: say what to do instead (M3 spec 4.1).
        if def.imported {
            return Diagnostic::new(
                codes::V0105,
                span,
                format!(
                    "field `{}` of `{}` is not `pub` in the Rust module `{module}`, so Varyk \
                     code cannot read or assign it",
                    decl.name, def.name
                ),
            )
            .with_label(decl.span, "declared here without plain `pub`")
            .with_note(
                "make the field `pub` in the Rust code, or add a `pub fn` to the struct's \
                 `impl` block there that does this",
            );
        }
        let diagnostic = Diagnostic::new(
            codes::V0105,
            span,
            format!(
                "field `{}` is private to module `{module}`, so it cannot be read or assigned \
                 from outside it",
                decl.name
            ),
        )
        .with_note(
            "make the field public with `pub`, or add a `pub fn` on the struct that does this",
        );
        declared_without_pub(diagnostic, decl.span, &decl.name)
    }

    /// V0105 at `span` (the field name as written in the literal) for a
    /// struct literal naming private field `field` of `struct_id` from
    /// outside its declaring module (spec 3.4): the same code as
    /// [`Symbols::private_field`], worded for a literal that cannot be
    /// written here at all.
    pub(crate) fn private_field_in_literal(
        &self,
        struct_id: StructId,
        field: usize,
        span: Span,
    ) -> Diagnostic {
        let def = &self.structs[struct_id.0 as usize];
        let decl = &def.fields[field];
        let module = &self.scopes[def.module.0 as usize].name;
        let diagnostic = Diagnostic::new(
            codes::V0105,
            span,
            format!(
                "`{} {{ .. }}` cannot be written here because `{}` is private to module \
                 `{module}`; add `pub` to the field or a `pub fn new`",
                def.name, decl.name
            ),
        );
        declared_without_pub(diagnostic, decl.span, &decl.name)
    }
}

/// V0105 at `span` for `what` (`function`, `type`, ...) `path`, used
/// outside its module although its declaration at `decl`, starting with
/// `keyword`, is not `pub`.
pub(crate) fn not_visible(
    what: &str,
    path: &str,
    span: Span,
    decl: Span,
    keyword: &str,
) -> Diagnostic {
    let diagnostic = Diagnostic::new(
        codes::V0105,
        span,
        format!(
            "{what} `{path}` is not marked `pub`, so it can only be used inside its own module and the modules within it"
        ),
    );
    declared_without_pub(diagnostic, decl, keyword)
}

/// Labels the `keyword` (`fn`, `struct`, `enum`, or `mod`) of the non-`pub` declaration at
/// `decl`, rather than underlining its whole body, and adds a `pub ` fix-it.
pub(crate) fn declared_without_pub(
    diagnostic: Diagnostic,
    decl: Span,
    keyword: &str,
) -> Diagnostic {
    let keyword_span = Span::new(decl.file, decl.start, decl.start + keyword.len() as u32);
    diagnostic
        .with_label(keyword_span, "declared here without `pub`")
        .with_fix_it(FixIt {
            span: Span::new(decl.file, decl.start, decl.start),
            replacement: "pub ".to_string(),
        })
}
