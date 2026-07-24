impl SectionedListSection for PermissionStudioSection {
    type Item = PermissionStudioItem;

    fn items(&self) -> &[Self::Item] {
        self.items.as_slice()
    }
}

use crate::{PermissionStudioItem, PermissionStudioSection};
use agena_tui_components::SectionedListSection;
