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
    minimumHeight: Math.max(panel.minHeight || 0, body.item ? body.item.minimumHeight : 0)   // -- WP LAY1b -- the panel's own
    fitBodies: body.item ? body.item.fitBodies : []
    // -- WP LAY1b --
    onClosing: if (body.item) body.item.closing()

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
    // -- WP LAY1b --
    Binding {
        target: body.item
        property: "minimized"
        value: window.visibility === Window.Minimized
        when: body.item !== null
    }
    Binding {
        target: window
        property: "font"
        value: body.item ? body.item.windowFont : undefined
        when: body.item !== null && body.item.windowFont !== undefined
    }
    Connections {
        target: body.item
        function onCloseRequested() { window.close() }
    }
}
