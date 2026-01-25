use solar::ast::DataLocation;
use solar::sema::{Gcx, Ty};

pub(crate) fn default_memory_if_ref<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> Ty<'gcx> {
    if ty.loc().is_none() && ty.is_reference_type() {
        ty.with_loc(gcx, DataLocation::Memory)
    } else {
        ty
    }
}
