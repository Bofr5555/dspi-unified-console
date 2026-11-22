# Object names: LAY3b (actions where they act)

Smoke sections: `lay3b-actions` (order 896) and `lay3b-menus` (897), `smoke/SmokeLay3b.cpp`. Screenshots `modern-*-menu*.png`, `modern-menu-*.png`.

- An ActionMenu's rows are `<menu>.<action id>`, made each time it opens; a submenu's or a dynamic row's `<menu>.<action id>.<name>`; the reason
  under a greyed row `<menu>.<action id>.why`. Menus: `AppMenu`, `TopBar.save.menu`, `TopBar.undo.menu`, `AutoEqWindow.menu`,
  `<device tabs>.menu.<device id>`.
- The right-click menus, children of the singleton `ContextMenus` (`engine->singletonInstance(...)->findChild`): `ContextMenus.channel`,
  `ContextMenus.band` (`.band.shape.<shape>`, `.band.slope.<order>`), `ContextMenus.graph`, `ContextMenus.slot` (`.slot.copyTo.<slot>`);
  `ContextMenus.clearQuestion` (in the window's content).
- `TopBar.undo` is a SplitButton now (`.button`, `.more`, `.menu`); `TopBar.history` is gone (its row is `TopBar.undo.menu.edit.history`).
- `AutoEqWindow.more`, `AutoEqWindow.menu` (Library > AutoEQ, in a page only).
