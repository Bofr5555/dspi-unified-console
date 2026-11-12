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
    Q_PROPERTY(int channel READ channel WRITE setChannel NOTIFY channelChanged)
    Q_PROPERTY(qreal level READ level WRITE setLevel NOTIFY levelChanged)
    Q_PROPERTY(bool clipping READ clipping WRITE setClipping NOTIFY clippingChanged)
    Q_PROPERTY(QColor color READ color WRITE setColor NOTIFY colorChanged)
    Q_PROPERTY(QColor trackColor READ trackColor WRITE setTrackColor NOTIFY trackColorChanged)
    Q_PROPERTY(QColor clipColor READ clipColor WRITE setClipColor NOTIFY clipColorChanged)
public:
    // Width of the clip mark at the right-hand end (the Mac's 3 pt)
    static constexpr qreal kClipZone = 3;

    explicit Meter(QQuickItem *parent = nullptr);

    int channel() const { return m_channel; }
    void setChannel(int channel);
    qreal level() const { return m_level; }
    // Clamped to 0..1
    void setLevel(qreal level);
    bool clipping() const { return m_clipping; }
    void setClipping(bool clipping);
    QColor color() const { return m_color; }
    void setColor(const QColor &color);
    QColor trackColor() const { return m_track; }
    void setTrackColor(const QColor &color);
    QColor clipColor() const { return m_clipColor; }
    void setClipColor(const QColor &color);

    // The bar's drawn width for the current size and level (tests)
    Q_INVOKABLE qreal barWidth() const;

signals:
    void channelChanged();
    void levelChanged();
    void clippingChanged();
    void colorChanged();
    void trackColorChanged();
    void clipColorChanged();

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
