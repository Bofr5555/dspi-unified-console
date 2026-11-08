import QtQuick 2.15

// WP QD22: keeps the keyboard focus in view. When the window's focus moves to
// an item a Flickable has scrolled out of sight (a Settings page, a long tool
// window), that Flickable scrolls just far enough to show it.
Item {
    id: follower
    property var window
    visible: false

    Connections {
        target: follower.window
        function onActiveFocusItemChanged() {
            var item = follower.window ? follower.window.activeFocusItem : null
            if (item) follower.scrollTo(item)
        }
    }

    function scrollTo(item) {
        for (var p = item.parent; p; p = p.parent) {
            if (p.contentY === undefined || p.contentItem === undefined || p.flickableDirection === undefined) continue
            if (p.contentHeight <= p.height && p.contentWidth <= p.width) continue
            var at = item.mapToItem(p.contentItem, 0, 0)
            if (p.contentHeight > p.height) {
                var lo = p.originY, hi = p.originY + p.contentHeight - p.height
                if (at.y - 8 < p.contentY) p.contentY = Math.max(lo, at.y - 8)
                else if (at.y + item.height + 8 > p.contentY + p.height) p.contentY = Math.min(hi, at.y + item.height + 8 - p.height)
            }
            if (p.contentWidth > p.width) {
                var left = p.originX, right = p.originX + p.contentWidth - p.width
                if (at.x - 8 < p.contentX) p.contentX = Math.max(left, at.x - 8)
                else if (at.x + item.width + 8 > p.contentX + p.width) p.contentX = Math.min(right, at.x + item.width + 8 - p.width)
            }
            return
        }
    }
}
