#ifndef METER_H
#define METER_H

#include <QColor>
#include <QMetaObject>
#include <QQuickItem>
#include <QtQml/qqmlregistration.h>

// A horizontal level bar drawn on the scene graph: the level from the left,
// and a short red mark at the right-hand end while the channel has clipped.
// It takes no clicks; the row around it owns them.
//
// With `channel` set to a wire channel, the bar follows the base's meter poll
// (Poller.peaks) by itself, so a reading moves this item and re-evaluates no
// QML binding. With `channel` at -1 the caller sets `level` (a tool window's
// own meter).
//
//     Meter { channel: 2; color: ChannelPalette.input(2); clipping: ...; height: 6 }
class Meter : public QQuickItem {
    Q_OBJECT
    QML_ELEMENT
    // Level 0..1 (clamped)
    Q_PROPERTY(qreal level READ level WRITE setLevel NOTIFY levelChanged)
    // The wire channel followed, -1 for none
    Q_PROPERTY(int channel MEMBER m_channel NOTIFY channelChanged)
    // How it looks: the bar, the track under it, the clip mark and whether it shows
    Q_PROPERTY(QColor color MEMBER m_color NOTIFY lookChanged)
    Q_PROPERTY(QColor trackColor MEMBER m_track NOTIFY lookChanged)
    Q_PROPERTY(QColor clipColor MEMBER m_clipColor NOTIFY lookChanged)
    Q_PROPERTY(bool clipping MEMBER m_clipping NOTIFY lookChanged)
public:
    // Width of the clip mark at the right-hand end (the Mac's 3 pt)
    static constexpr qreal kClipZone = 3;

    explicit Meter(QQuickItem *parent = nullptr);

    qreal level() const { return m_level; }
    void setLevel(qreal level);

    // The bar's drawn width for the current size and level (tests)
    Q_INVOKABLE qreal barWidth() const;

signals:
    void levelChanged();
    void channelChanged();
    void lookChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *old, UpdatePaintNodeData *) override;
    void geometryChange(const QRectF &now, const QRectF &before) override;
    void componentComplete() override;

private:
    void follow();
    void readPeak();

    int m_channel = -1;
    qreal m_level = 0;
    bool m_clipping = false;
    QColor m_color;
    QColor m_track = Qt::transparent;
    QColor m_clipColor;
    QMetaObject::Connection m_feed;
};

#endif // METER_H
