#ifndef RTABARS_H
#define RTABARS_H

#include <QColor>
#include <QElapsedTimer>
#include <QFont>
#include <QQuickPaintedItem>
#include <QtQml/qqmlregistration.h>

#include "RtaSurface.h"

class RtaController;

// -- WP QP9 -- One channel's spectrum as bars (the Mac's RTA bars): a bar per
// resolved band from the analyser's floor up to its level, a gradient from
// the top down, a cap at the recent peak, frequency labels under the bars and
// optionally the dB grid with its labels. Bands that do not resolve at this
// rate and transform size are left out; the rest share the width.
class RtaBars : public QQuickPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    // The channel at `tap`
    Q_PROPERTY(int channel MEMBER m_channel NOTIFY lookChanged)
    Q_PROPERTY(int tap MEMBER m_tap NOTIFY lookChanged)
    Q_PROPERTY(QColor color MEMBER m_color NOTIFY lookChanged)
    Q_PROPERTY(bool frequencyLabels MEMBER m_freqLabels NOTIFY lookChanged)
    Q_PROPERTY(bool dbLabels MEMBER m_dbLabels NOTIFY lookChanged)
    Q_PROPERTY(QColor gridColor MEMBER m_gridColor NOTIFY lookChanged)
    Q_PROPERTY(QColor labelColor MEMBER m_labelColor NOTIFY lookChanged)
    Q_PROPERTY(QFont labelFont MEMBER m_labelFont NOTIFY lookChanged)
    Q_PROPERTY(bool active MEMBER m_active NOTIFY lookChanged)
    // Bars drawn above the floor in the last paint (tests)
    Q_PROPERTY(int drawnBars READ drawnBars NOTIFY painted)
public:
    explicit RtaBars(QQuickItem *parent = nullptr);
    void paint(QPainter *painter) override;
    int drawnBars() const { return m_drawn; }

signals:
    void lookChanged();
    void painted();

private:
    RtaController *m_rta = nullptr;
    int m_channel = 0;
    int m_tap = 1;
    QColor m_color = Qt::white;
    bool m_freqLabels = true;
    bool m_dbLabels = false;
    QColor m_gridColor = Qt::gray;
    QColor m_labelColor = Qt::gray;
    QFont m_labelFont;
    bool m_active = true;
    RtaGlide m_avg, m_peak;
    int m_seriesKey = -1;
    QElapsedTimer m_clock;
    int m_drawn = 0;
};

#endif // RTABARS_H
