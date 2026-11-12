#ifndef RTABARS_H
#define RTABARS_H

#include <QColor>

#include "RtaSurface.h"

// -- WP QP9 -- One channel's spectrum as bars (the Mac's RTA bars): a bar per
// resolved band from the floor up to its level, shaded lighter towards the
// bottom, a thin cap at the recent peak, frequency labels underneath and,
// with `dbLabels`, the dB lines and their numbers. Bands that do not resolve
// at this rate and transform size are left out and the rest share the width.
class RtaBars : public RtaPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    // The channel at `tap`
    Q_PROPERTY(int channel MEMBER m_channel NOTIFY lookChanged)
    Q_PROPERTY(QColor color MEMBER m_color NOTIFY lookChanged)
    Q_PROPERTY(bool frequencyLabels MEMBER m_frequencyLabels NOTIFY lookChanged)
    Q_PROPERTY(bool dbLabels MEMBER m_dbLabels NOTIFY lookChanged)
    // Bars above the floor in the last paint (tests)
    Q_PROPERTY(int drawnBars READ drawnBars NOTIFY painted)
public:
    explicit RtaBars(QQuickItem *parent = nullptr);
    void paint(QPainter *painter) override;
    int drawnBars() const { return m_drawn; }

private:
    // The labels under the bars: decades first, then the others where they fit
    void paintFrequencies(QPainter &g, const QRectF &plot, const std::vector<int> &bands, double slot) const;

    int m_channel = 0;
    QColor m_color = Qt::white;
    bool m_frequencyLabels = true;
    bool m_dbLabels = false;
    RtaGlide m_level, m_hold;
    int m_series = -1;
    int m_drawn = 0;
};

#endif // RTABARS_H
