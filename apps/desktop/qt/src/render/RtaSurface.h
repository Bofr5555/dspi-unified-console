#ifndef RTASURFACE_H
#define RTASURFACE_H

#include <QColor>
#include <QElapsedTimer>
#include <QFont>
#include <QHash>
#include <QList>
#include <QQuickPaintedItem>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <vector>

class RtaController;

// -- WP QP9 -- The glide between the analyser's frames (the Mac's display
// interpolation): one value per slot follows its target with a one-pole
// filter, rising faster than it falls; peaks rise at once. A change of the
// series (another channel, another number of slots) snaps.
class RtaGlide {
public:
    // Steps every slot towards `targets` over `dt` seconds with time constant
    // `tau` (0 jumps); true while some slot is still moving
    bool step(const std::vector<double> &targets, double dt, double tau, bool peak);
    const std::vector<double> &values() const { return m_values; }
    void reset() { m_values.clear(); }

private:
    std::vector<double> m_values;
};

// -- WP QP9 -- The live spectrum as curves (the Mac's GraphSpectrumOverlay and
// the analyser window's curve view): per channel a translucent fill down to
// the bottom, an edge line, the peak contour, on a log frequency axis
// (`minFreq`..`maxFreq`) and the analyser's own dB scale (floor at the
// bottom, ceiling at the top). With one channel the bass bands blend into the
// transform's bins between 200 and 400 Hz. With `grid` it draws its own grid
// and labels (the window); on the response graph the graph's grid shows.
// It takes no input: the graph's handlers stay in charge.
class RtaSurface : public QQuickPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    // Channels at the snapshot's tap, and a colour each
    Q_PROPERTY(QList<int> channels MEMBER m_channels NOTIFY lookChanged)
    Q_PROPERTY(QVariantList colors MEMBER m_colors NOTIFY lookChanged)
    Q_PROPERTY(int tap MEMBER m_tap NOTIFY lookChanged)
    Q_PROPERTY(double minFreq MEMBER m_minFreq NOTIFY lookChanged)
    Q_PROPERTY(double maxFreq MEMBER m_maxFreq NOTIFY lookChanged)
    Q_PROPERTY(bool grid MEMBER m_grid NOTIFY lookChanged)
    Q_PROPERTY(bool glow MEMBER m_glow NOTIFY lookChanged)
    Q_PROPERTY(QColor gridColor MEMBER m_gridColor NOTIFY lookChanged)
    Q_PROPERTY(QColor labelColor MEMBER m_labelColor NOTIFY lookChanged)
    Q_PROPERTY(QFont labelFont MEMBER m_labelFont NOTIFY lookChanged)
    // Draws and glides only while on (a hidden window, a hidden slot)
    Q_PROPERTY(bool active MEMBER m_active NOTIFY lookChanged)
    // Channels drawn in the last paint (tests)
    Q_PROPERTY(int drawnCount READ drawnCount NOTIFY painted)
public:
    explicit RtaSurface(QQuickItem *parent = nullptr);
    void paint(QPainter *painter) override;
    int drawnCount() const { return m_drawn; }

    // The mapping
    double xFor(double hz, double plotX, double plotW) const;

signals:
    void lookChanged();
    void painted();

private:
    void follow();

    RtaController *m_rta = nullptr;
    QList<int> m_channels;
    QVariantList m_colors;
    int m_tap = 1;
    double m_minFreq = 20, m_maxFreq = 20000;
    bool m_grid = false;
    bool m_glow = false;
    bool m_active = true;
    QColor m_gridColor = Qt::gray;
    QColor m_labelColor = Qt::gray;
    QFont m_labelFont;
    QHash<int, RtaGlide> m_avg, m_peak;
    RtaGlide m_bins;
    int m_binsKey = -1;
    QElapsedTimer m_clock;
    int m_drawn = 0;
};

#endif // RTASURFACE_H
