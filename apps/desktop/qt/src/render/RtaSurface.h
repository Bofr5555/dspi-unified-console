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

class QPainter;
class RtaController;

// -- WP QP9 -- The glide between the analyser's frames (the Mac's display
// interpolation): each slot approaches its target exponentially, rising in
// 0.4 of the time it takes to fall; a peak rises at once. A different number
// of slots starts over without a glide.
class RtaGlide {
public:
    // Moves every slot towards `targets` by `dt` seconds with time constant
    // `tau` (0: at once); true while a slot has not arrived
    bool advance(const std::vector<double> &targets, double dt, double tau, bool peak);
    const std::vector<double> &values() const { return m_now; }
    void forget() { m_now.clear(); }

private:
    std::vector<double> m_now;
};

// -- WP QP9 -- What the spectrum's two renderers share: the controller they
// read, the dB scale, the grid with its labels, and the clock their glides
// run on. Neither takes pointer input, hover included: the graph's own
// handlers stay in charge underneath.
class RtaPaintedItem : public QQuickPaintedItem {
    Q_OBJECT
    QML_ANONYMOUS
    Q_PROPERTY(int tap MEMBER m_tap NOTIFY lookChanged)
    Q_PROPERTY(QColor gridColor MEMBER m_gridColor NOTIFY lookChanged)
    Q_PROPERTY(QColor labelColor MEMBER m_labelColor NOTIFY lookChanged)
    Q_PROPERTY(QFont labelFont MEMBER m_labelFont NOTIFY lookChanged)
    // Draws and glides only while on (a hidden window, an empty slot)
    Q_PROPERTY(bool active MEMBER m_active NOTIFY lookChanged)
public:
    explicit RtaPaintedItem(QQuickItem *parent = nullptr);

    // The frequencies with a label on the axes: 1, 2 and 5 of each decade
    // from 10 Hz, and 20 kHz
    static const std::vector<double> &labelledHz();
    // Height of the frequency labels' row under a plot
    static constexpr double kLabelRow = 12;

signals:
    void lookChanged();
    void painted();

protected:
    // 0..1 of the scale's height for a level in dB
    double heightOf(double db) const;
    // The horizontal dB lines every 12 dB under the ceiling, the 0 dB one
    // stronger, with their numbers at the right when `numbers`
    void paintLevels(QPainter &g, const QRectF &plot, bool numbers) const;
    // Seconds since the last paint (for the glides), at most a quarter second
    double tick();
    // After a paint: tells QML the count changed, and asks for another
    // frame while a glide moves (never from inside paint itself)
    void afterPaint(bool countChanged, bool moving);

    RtaController *m_rta = nullptr;
    int m_tap = 1;
    QColor m_gridColor = Qt::gray;
    QColor m_labelColor = Qt::gray;
    QFont m_labelFont;
    bool m_active = true;

private:
    QElapsedTimer m_clock;
};

// -- WP QP9 -- The live spectrum as curves (the Mac's GraphSpectrumOverlay and
// the analyser window's curve view): per channel a fill down to the bottom,
// fading to nothing, an edge line and the peak contour, on a log frequency
// axis (`minFreq`..`maxFreq`) and the analyser's own scale (floor at the
// bottom, ceiling at the top). One channel shows its bass bands blended into
// the transform's bins between 200 and 400 Hz. With `grid` it draws its own
// grid and labels (the window); on the response graph the graph's shows.
class RtaSurface : public RtaPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    // Channels at `tap`, and a colour each
    Q_PROPERTY(QList<int> channels MEMBER m_channels NOTIFY lookChanged)
    Q_PROPERTY(QVariantList colors MEMBER m_colors NOTIFY lookChanged)
    Q_PROPERTY(double minFreq MEMBER m_minFreq NOTIFY lookChanged)
    Q_PROPERTY(double maxFreq MEMBER m_maxFreq NOTIFY lookChanged)
    Q_PROPERTY(bool grid MEMBER m_grid NOTIFY lookChanged)
    Q_PROPERTY(bool glow MEMBER m_glow NOTIFY lookChanged)
    // Channels drawn in the last paint (tests)
    Q_PROPERTY(int drawnCount READ drawnCount NOTIFY painted)
public:
    explicit RtaSurface(QQuickItem *parent = nullptr);
    void paint(QPainter *painter) override;
    int drawnCount() const { return m_drawn; }

private:
    double xAt(double hz, const QRectF &plot) const;

    QList<int> m_channels;
    QVariantList m_colors;
    double m_minFreq = 20, m_maxFreq = 20000;
    bool m_grid = false;
    bool m_glow = false;
    QHash<int, RtaGlide> m_level, m_hold;
    RtaGlide m_fine;
    int m_fineFor = -1;
    int m_drawn = 0;
};

#endif // RTASURFACE_H
