import QtQuick
import QtQml
import UConsole

// -- WP QP12 -- "Add Control" / "Add Output" (the Mac's add menus): a button
// opening the component types `types` offers; picking one adds its card.
AppButton {
    id: button
    property var types: []
    iconName: "plus"
    enabled: CsController.writable && types.length > 0 && CsController.canAdd("binding")
    onClicked: menu.popup(button, 0, button.height)

    AppMenu {
        id: menu
        objectName: button.objectName.length ? button.objectName + ".menu" : ""
        Instantiator {
            model: button.types
            delegate: AppMenuItem {
                required property var modelData
                objectName: button.objectName.length ? button.objectName + ".type." + modelData.type : ""
                text: modelData.name
                onTriggered: CsController.addControl(modelData.type)
            }
            onObjectAdded: (index, object) => menu.insertItem(index, object)
            onObjectRemoved: (index, object) => menu.removeItem(object)
        }
    }
}
