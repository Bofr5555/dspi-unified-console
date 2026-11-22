import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The dashboard layout popover (the Mac's DashboardLayoutPanel): Auto, or a
// fixed number of cards per row. Every card's gear opens one; they all edit
// the same setting. Opened from its anchor button, which is its parent, so a
// click on that button while it is open closes it once (behaviour.md §26).
// -- WP LAY2 -- The content is DashboardLayoutContent, which Modern shows as
// well.
PopoverPanel {
    id: popup
    objectName: "DashboardLayoutPopup"
    title: qsTr("Dashboard layout")
    width: 220
    padding: 0
    // Kept inside the window when its gear is near an edge
    margins: Theme.spacingSmall
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent

    contentItem: DashboardLayoutContent {}
}
