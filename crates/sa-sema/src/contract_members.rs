use solar::interface::Symbol;
use solar::sema::ty::{Ty, TyKind};
use solar::sema::{Gcx, hir};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractMemberAccess {
    Call,
    Value,
}

#[derive(Clone, Copy, Debug)]
pub struct ContractMember<'gcx> {
    pub name: Symbol,
    pub item_id: hir::ItemId,
    pub ty: Ty<'gcx>,
}

pub fn contract_id_from_type(ty: Ty<'_>) -> Option<hir::ContractId> {
    match ty.kind {
        TyKind::Type(inner) => match inner.kind {
            TyKind::Contract(contract_id) => Some(contract_id),
            _ => None,
        },
        _ => None,
    }
}

pub fn contract_type_members<'gcx>(
    gcx: Gcx<'gcx>,
    contract_id: hir::ContractId,
    base_accessible: bool,
    access: ContractMemberAccess,
) -> Vec<ContractMember<'gcx>> {
    let contract = gcx.hir.contract(contract_id);
    let is_library = contract.kind.is_library();
    let mut members = Vec::new();

    let mut push_member = |item_id| {
        let name = gcx.item_name(item_id).name;
        let ty = gcx.type_of_item(item_id);
        members.push(ContractMember { name, item_id, ty });
    };

    for interface_func in gcx.interface_functions(contract_id).own().iter() {
        let func = gcx.hir.function(interface_func.id);
        if func.name.is_none() || !func.kind.is_ordinary() {
            continue;
        }
        if allow_function(func.visibility, is_library, base_accessible, access) {
            push_member(hir::ItemId::from(interface_func.id));
        }
    }

    for &item_id in contract.items {
        match item_id {
            hir::ItemId::Function(func_id) => {
                let func = gcx.hir.function(func_id);
                if func.name.is_none() || !func.kind.is_ordinary() {
                    continue;
                }
                if func.visibility != hir::Visibility::Internal {
                    continue;
                }
                if allow_function(func.visibility, is_library, base_accessible, access) {
                    push_member(item_id);
                }
            }
            hir::ItemId::Variable(var_id) => {
                if access == ContractMemberAccess::Call {
                    continue;
                }
                let var = gcx.hir.variable(var_id);
                if var.name.is_none() || var.kind != hir::VarKind::State || !var.is_constant() {
                    continue;
                }
                let visibility = var.visibility.unwrap_or(hir::Visibility::Internal);
                if allow_constant(visibility, is_library, base_accessible) {
                    push_member(item_id);
                }
            }
            _ => {}
        }
    }

    members
}

fn allow_function(
    visibility: hir::Visibility,
    is_library: bool,
    base_accessible: bool,
    access: ContractMemberAccess,
) -> bool {
    if is_library {
        return visibility >= hir::Visibility::Internal;
    }

    match access {
        ContractMemberAccess::Call => {
            base_accessible
                && visibility >= hir::Visibility::Internal
                && visibility <= hir::Visibility::Public
        }
        ContractMemberAccess::Value => {
            visibility >= hir::Visibility::Public
                || (base_accessible && visibility == hir::Visibility::Internal)
        }
    }
}

fn allow_constant(visibility: hir::Visibility, is_library: bool, base_accessible: bool) -> bool {
    if is_library {
        return visibility >= hir::Visibility::Internal;
    }
    visibility >= hir::Visibility::Public
        || (base_accessible && visibility == hir::Visibility::Internal)
}
