import QtQuick
import UConsole

// -- WP LAY0 -- The one window every panel opens in (MODLAY 4.1): a ToolWindow
// around a Panel, made by Shell for an action that has a PanelTable row. Its
// objectName is the type Registrar::Window registered for the action
// ("StatsWindow"), so finding a window by name works as before; its title,
// sizes and remembered place (windows/<windowId>) come from the row.
ToolWindow {
    id: window
    // The action's PanelTable row (Shell sets it): {action, panel, windowId,
    // title, width, height, minWidth, minHeight, ...}
    required property var panel
    readonly property Item content: body.item

    windowId: panel.windowId
    title: panel.title
    defaultWidth: panel.width
    defaultHeight: panel.height
    minimumWidth: panel.minWidth || 0
    minimumHeight: panel.minHeight || 0
    fitBodies: body.item ? body.item.fitBodies : []

    Loader {
        id: body
        objectName: "PanelWindow.body"
        anchors.fill: parent
        sourceComponent: Qt.createComponent("UConsole", window.panel.panel)
        onStatusChanged: if (status === Loader.Error) console.warn("PanelWindow: the panel", window.panel.panel, "did not load")
    }
    Binding {
        target: body.item
        property: "shown"
        value: window.visible
        when: body.item !== null
    }
    Binding {
        target: body.item
        property: "locked"
        value: window.senderLocked
        when: body.item !== null
    }
    Binding {
        target: body.item
        property: "actionId"
        value: window.actionId
        when: body.item !== null
    }
}
