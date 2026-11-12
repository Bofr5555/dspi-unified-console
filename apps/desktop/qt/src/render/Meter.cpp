#include "Meter.h"

#include "Poller.h"

#include <QQuickWindow>
#include <QSGRectangleNode>

Meter::Meter(QQuickItem *parent) : QQuickItem(parent) {
    setFlag(ItemHasContents, true);
    setAcceptedMouseButtons(Qt::NoButton);
}

void Meter::setChannel(int channel) {
    if (channel == m_channel) return;
    m_channel = channel;
    follow();
    emit channelChanged();
}

void Meter::setLevel(qreal level) {
    const qreal clamped = qBound(0.0, level, 1.0);
    if (qFuzzyCompare(clamped + 1, m_level + 1)) return;
    m_level = clamped;
    update();
    emit levelChanged();
}

void Meter::setClipping(bool clipping) {
    if (clipping == m_clipping) return;
    m_clipping = clipping;
    update();
    emit clippingChanged();
}

void Meter::setColor(const QColor &color) {
    if (color == m_color) return;
    m_color = color;
    update();
    emit colorChanged();
}

void Meter::setTrackColor(const QColor &color) {
    if (color == m_track) return;
    m_track = color;
    update();
    emit trackColorChanged();
}

void Meter::setClipColor(const QColor &color) {
    if (color == m_clipColor) return;
    m_clipColor = color;
    update();
    emit clipColorChanged();
}

qreal Meter::barWidth() const { return m_level * qMax(0.0, width() - kClipZone); }

void Meter::componentComplete() {
    QQuickItem::componentComplete();
    follow();
}

// Subscribes to the meter poll for a wire channel, or drops the subscription
void Meter::follow() {
    QObject::disconnect(m_feed);
    Poller *poller = Poller::instance();
    if (m_channel < 0 || !poller || !isComponentComplete()) return;
    m_feed = connect(poller, &Poller::metersChanged, this, &Meter::readPeak);
    readPeak();
}

void Meter::readPeak() {
    const Poller *poller = Poller::instance();
    if (!poller) return;
    const QVariantList peaks = poller->peaks();
    // A channel the reading does not carry shows nothing rather than a guess
    setLevel(m_channel < peaks.size() ? peaks.at(m_channel).toReal() : 0.0);
}

void Meter::geometryChange(const QRectF &now, const QRectF &before) {
    QQuickItem::geometryChange(now, before);
    if (now.size() != before.size()) update();
}

QSGNode *Meter::updatePaintNode(QSGNode *old, UpdatePaintNodeData *) {
    QQuickWindow *w = window();
    if (!w) return old;
    // Three child rectangles, made once: the track, the bar, the clip mark
    QSGNode *root = old;
    if (!root) {
        root = new QSGNode;
        for (int i = 0; i < 3; ++i) {
            QSGRectangleNode *rect = w->createRectangleNode();
            root->appendChildNode(rect);
        }
    }
    auto *track = static_cast<QSGRectangleNode *>(root->childAtIndex(0));
    auto *bar = static_cast<QSGRectangleNode *>(root->childAtIndex(1));
    auto *clip = static_cast<QSGRectangleNode *>(root->childAtIndex(2));
    const qreal h = height();
    const qreal meterWidth = qMax(0.0, width() - kClipZone);
    track->setRect(0, 0, width(), h);
    track->setColor(m_track);
    bar->setRect(0, 0, m_level * meterWidth, h);
    bar->setColor(m_color);
    clip->setRect(meterWidth, 0, m_clipping ? kClipZone : 0, h);
    clip->setColor(m_clipColor);
    return root;
}
