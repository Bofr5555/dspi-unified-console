import QtQuick 2.15
import QtQuick.Controls 2.15

// A numeric value with its unit, editable in place (macOS-style): plain text
// at rest, a soft rounded fill on hover, and a filled field with an accent
// ring while editing. Return commits, Esc cancels.
Row {
    id: valueFieldRoot
    spacing: 4
    height: 24

    property real value: 0
    property string suffix: ""
    property int decimals: 1
    // Up to this many decimals show when the value has them ("0.00##" for
    // decimals 2, maxDecimals 4); typed values keep them
    property int maxDecimals: decimals
    property real minValue: -999
    property real maxValue: 999
    property int fieldWidth: 60
    property int fontSize: 13
    property color textColor: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
    property color unitColor: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45)
    property int unitSize: Math.max(10, fontSize - 2)
    // Values at or below this read as -∞ (e.g. the master volume mute sentinel)
    property real infinityAt: -1e9
    // A value that means "off" reads as offText (e.g. a level at its -30 dB floor)
    property real offAt: NaN
    property string offText: qsTr("Off")
    // Compact fields (Matrix Mixer) adjust on plain scrolling; otherwise
    // Ctrl+scroll steps the value and plain scrolling scrolls the page
    property bool plainWheel: false
    property real wheelStep: Math.pow(10, -decimals)
    // -- WP SL1 -- The wheel lands on multiples of wheelStep: from a value
    // between two (a fractional delay) it goes to the next one up or down
    property bool wheelOnGrid: false

    signal valueEdited(real newValue)
    // -- WP QD22 -- What a screen reader calls the field (the row's label)
    property string accessibleName: ""

    // Start typing in the field, its value selected
    function beginEdit() { textField.forceActiveFocus() }
    readonly property bool editing: textField.activeFocus

    TextField {
        id: textField
        width: fieldWidth
        height: parent.height
        font.pixelSize: valueFieldRoot.fontSize
        color: isMacOS ? (textField.activeFocus ? MacColors.accent : valueFieldRoot.textColor) : valueFieldRoot.textColor
        selectionColor: isMacOS ? MacColors.selectedControl : "#0a7cff"
        selectedTextColor: "white"
        horizontalAlignment: Text.AlignRight
        verticalAlignment: Text.AlignVCenter
        selectByMouse: true
        Accessible.name: valueFieldRoot.accessibleName
        Accessible.description: valueFieldRoot.suffix
        hoverEnabled: true
        leftPadding: 6
        rightPadding: 6
        topPadding: 0
        bottomPadding: 0

        background: Rectangle {
            radius: 5
            color: isMacOS ? "transparent" : textField.activeFocus ? Qt.rgba(1, 1, 1, 0.10)
                 : textField.hovered && textField.enabled ? Qt.rgba(1, 1, 1, 0.06) : "transparent"
            border.width: textField.activeFocus ? 1.5 : 0
            border.color: isMacOS ? MacColors.keyboardFocusIndicator : "#0a7cff"
            Behavior on color { ColorAnimation { duration: 90 } }
        }

        text: formatValue(value)

        function formatValue(v) {
            if (!isNaN(offAt) && Math.abs(v - offAt) < 1e-4) return offText
            if (v <= infinityAt) return "-∞"
            var s = v.toFixed(Math.max(decimals, maxDecimals))
            for (var extra = maxDecimals - decimals; extra > 0 && s.charAt(s.length - 1) === "0"; extra--)
                s = s.slice(0, -1)
            return s.charAt(s.length - 1) === "." ? s.slice(0, -1) : s
        }

        onEditingFinished: {
            var cleaned = text.replace(suffix, "").trim()
            var parsed = parseFloat(cleaned)
            // Only a real edit commits: focus leaving an untouched field must
            // not write back (or clamp) a value set elsewhere, e.g. +12 dB
            // from a control surface in a field that stops at +10
            if (!isNaN(parsed) && cleaned !== formatValue(valueFieldRoot.value)) {
                parsed = Math.max(minValue, Math.min(maxValue, parsed))
                valueFieldRoot.valueEdited(parsed)
            }
            text = formatValue(valueFieldRoot.value)
        }

        onActiveFocusChanged: {
            text = formatValue(value)
            if (activeFocus) selectAll()
        }

        // Esc drops the edit and the focus
        Keys.onEscapePressed: {
            text = formatValue(valueFieldRoot.value)
            focus = false
        }

        Connections {
            target: valueFieldRoot
            function onValueChanged() {
                if (!textField.activeFocus)
                    textField.text = textField.formatValue(valueFieldRoot.value)
            }
        }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.NoButton
            onWheel: {
                if (!plainWheel && !(wheel.modifiers & Qt.ControlModifier)) { wheel.accepted = false; return }
                var up = wheel.angleDelta.y > 0
                var target = value + (up ? wheelStep : -wheelStep)
                if (wheelOnGrid) {
                    var n = value / wheelStep
                    // 1e-6: a value a float's error off a step counts as on it
                    target = (up ? Math.floor(n + 1e-6) + 1 : Math.ceil(n - 1e-6) - 1) * wheelStep + 0
                }
                // A value already outside the range steps from where it is
                // rather than jumping to the limit
                var lo = Math.min(minValue, value), hi = Math.max(maxValue, value)
                var newVal = Math.max(lo, Math.min(hi, target))
                if (newVal !== value) valueFieldRoot.valueEdited(newVal)
            }
        }
    }

    Text {
        text: suffix
        font.pixelSize: valueFieldRoot.unitSize
        color: valueFieldRoot.unitColor
        anchors.verticalCenter: parent.verticalCenter
        width: Math.max(20, implicitWidth)   // room for longer units (dBFS)
        visible: suffix !== ""
    }
}
