#ifndef POINTERTRACKER_H
#define POINTERTRACKER_H

#include <QQuickItem>
#include <QPointer>
#include <QDeadlineTimer>

// Whether the pointer is over this item, whatever is on top of it. Qt Quick
// hands hover to the topmost item that takes it, so a HoverHandler or
// MouseArea under the graph editor or a band chip misses the pointer coming
// in; this watches the window's own pointer moves instead. `containsPointer`
// changes only when the pointer crosses the item's edge.
class PointerTracker : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(bool containsPointer READ containsPointer NOTIFY containsPointerChanged)

public:
    explicit PointerTracker(QQuickItem *parent = nullptr);
    bool containsPointer() const { return m_inside; }
    // -- WP QD40 -- The window's last pointer move, sent to it again where the
    // pointer still is. Qt 6.5 to 6.8 set the cursor only when the pointer moves,
    // so an item with a cursor that appears under a still pointer (a popover's
    // area over its anchor) shows it only after this; Qt 6.9 and newer need none.
    // Opening a popup window sends this window a Leave, so whether the pointer
    // is in it is asked before the popup shows, and the move sent after; a Leave
    // that arrives after it, for a short while, is followed by the move again.
    Q_INVOKABLE bool pointerInWindow() const { return m_inWindow; }
    Q_INVOKABLE void resendPointer();

signals:
    void containsPointerChanged();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;

private:
    void setInside(bool inside);
    void sendLastMove();

    QPointer<QQuickWindow> m_window;
    bool m_inside = false;
    // -- WP QD40 -- Where the pointer was last seen in the window, and whether
    // it is in it now
    bool m_seen = false;
    bool m_inWindow = false;
    QPointF m_lastScene;
    QPointF m_lastGlobal;
    QDeadlineTimer m_resendFor;   // a resend's while: a Leave in it is the popup's
};

#endif // POINTERTRACKER_H
