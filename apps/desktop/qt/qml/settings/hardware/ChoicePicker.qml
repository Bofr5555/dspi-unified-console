import QtQuick
import UConsole

// A popup of [{value, text}] showing `value`'s entry; `chosen(value)` fires
// on a person's choice only. A value not in the list shows as its own text
// (`missingText`), so the picker never shows something it is not.
AppComboBox {
    id: picker
    property var choices: []
    property var value
    property string missingText
    signal chosen(var value)

    readonly property int found: {
        for (let i = 0; i < choices.length; ++i)
            if (choices[i].value === value) return i
        return -1
    }
    readonly property var shown: found >= 0 || missingText.length === 0
                                 ? choices : choices.concat([{ value: value, text: missingText }])

    implicitWidth: 120
    model: shown.map(c => c.text)
    currentIndex: found >= 0 ? found : shown.length - 1
    onActivated: index => {
        const c = shown[index]
        if (c && c.value !== value) picker.chosen(c.value)
    }
}
