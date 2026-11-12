#ifndef BODEPLOT_H
#define BODEPLOT_H

#include <QColor>
#include <QElapsedTimer>
#include <QFont>
#include <QHash>
#include <QList>
#include <QPointer>
#include <QQuickPaintedItem>
#include <QTimer>
#include <QVector>
#include <QtQml/qqmlregistration.h>

class GraphController;

// -- WP QP3 -- The response graph's drawing (the Mac's BodePlotView canvases
// and BodeLineShape): the grid, the curves with the phase trace, and the axis
// labels. One item draws the layers named in `layers`, so GraphPlot.qml can
// stack the grid, the analyser's slot, the curves, the labels and the editor's
// slot in the Mac's order with three of these.
//
// The curves are GraphController's (the core's); this item only maps them to
// the plot: x by log frequency between the graph's limits, y by dB between
// its top and bottom. Curves that are equal are one line in a gradient of
// their channels' colours; with no visible curve active (the pop-out while it
// does not follow the page) they are dashed. A changed curve moves to its new
// shape over a fifth of a second.
class BodePlot : public QQuickPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(GraphController *graph READ graph WRITE setGraph NOTIFY graphChanged)
    Q_PROPERTY(int layers READ layers WRITE setLayers NOTIFY layersChanged)
    Q_PROPERTY(QList<int> channels READ channels WRITE setChannels NOTIFY channelsChanged)
    Q_PROPERTY(int activeChannel READ activeChannel WRITE setActiveChannel NOTIFY channelsChanged)
    // Main window and a following pop-out: the page's channel is the active
    // curve and the others are dashed only without one
    Q_PROPERTY(bool followsSelection READ followsSelection WRITE setFollowsSelection NOTIFY channelsChanged)
    // Leave out GraphController.overlayChannel, which the PEQ overlay draws
    Q_PROPERTY(bool excludeOverlay READ excludeOverlay WRITE setExcludeOverlay NOTIFY channelsChanged)
    // Colours by wire channel (ChannelPalette's, from QML)
    Q_PROPERTY(QVariantList channelColors READ channelColors WRITE setChannelColors NOTIFY styleChanged)
    Q_PROPERTY(QColor gridColor READ gridColor WRITE setGridColor NOTIFY styleChanged)
    Q_PROPERTY(QColor labelColor READ labelColor WRITE setLabelColor NOTIFY styleChanged)
    Q_PROPERTY(QColor phaseColor READ phaseColor WRITE setPhaseColor NOTIFY styleChanged)
    Q_PROPERTY(QFont labelFont READ labelFont WRITE setLabelFont NOTIFY styleChanged)
    Q_PROPERTY(bool animated READ animated WRITE setAnimated NOTIFY styleChanged)
    // What the last paint drew (the smoke driver reads these)
    Q_PROPERTY(QList<int> drawnChannels READ drawnChannels NOTIFY drawn)
    Q_PROPERTY(int drawnLines READ drawnLines NOTIFY drawn)
    Q_PROPERTY(bool drawnPhase READ drawnPhase NOTIFY drawn)
    Q_PROPERTY(bool animating READ animating NOTIFY drawn)
public:
    enum Layer { Grid = 1, Curves = 2, Labels = 4 };
    Q_ENUM(Layer)

    explicit BodePlot(QQuickItem *parent = nullptr);

    GraphController *graph() const { return m_graph; }
    void setGraph(GraphController *graph);
    int layers() const { return m_layers; }
    void setLayers(int layers);
    QList<int> channels() const { return m_channels; }
    void setChannels(const QList<int> &channels);
    int activeChannel() const { return m_active; }
    void setActiveChannel(int channel);
    bool followsSelection() const { return m_follows; }
    void setFollowsSelection(bool on);
    bool excludeOverlay() const { return m_excludeOverlay; }
    void setExcludeOverlay(bool on);
    QVariantList channelColors() const { return m_colorList; }
    void setChannelColors(const QVariantList &colors);
    QColor gridColor() const { return m_gridColor; }
    void setGridColor(const QColor &c);
    QColor labelColor() const { return m_labelColor; }
    void setLabelColor(const QColor &c);
    QColor phaseColor() const { return m_phaseColor; }
    void setPhaseColor(const QColor &c);
    QFont labelFont() const { return m_font; }
    void setLabelFont(const QFont &f);
    bool animated() const { return m_animated; }
    void setAnimated(bool on);
    QList<int> drawnChannels() const { return m_drawnChannels; }
    int drawnLines() const { return m_drawnLines; }
    bool drawnPhase() const { return m_drawnPhase; }
    bool animating() const { return m_timer.isActive(); }

    // The plot's mapping, for the overlays: item x for a frequency in Hz and
    // back, item y for a level in dB and back
    Q_INVOKABLE double xForFrequency(double hz) const;
    Q_INVOKABLE double frequencyForX(double x) const;
    Q_INVOKABLE double yForDb(double db) const;
    Q_INVOKABLE double dbForY(double y) const;
    // The dB grid's step for the current span (the labels use the same)
    Q_INVOKABLE double dbStep() const;

    // Runs in the scene graph's sync (the GUI thread waits): reads, never writes
    void paint(QPainter *painter) override;

protected:
    void geometryChange(const QRectF &now, const QRectF &before) override;

signals:
    void graphChanged();
    void layersChanged();
    void channelsChanged();
    void styleChanged();
    void drawn();

private:
    struct Line {
        QVector<double> from, to, shown;
    };
    // Sets `field`; true when that changed it
    template <class T>
    static bool change(T &field, const T &value) {
        if (field == value) return false;
        field = value;
        return true;
    }
    void restyle();
    void retarget();
    void tick();
    // Where `hz` falls between the graph's limits (0 to 1, log frequency), and
    // `db` between `top` (0) and `bottom` (1)
    double freqFraction(double hz) const;
    static double levelFraction(double db, double top, double bottom);
    void paintGrid(QPainter *p);
    void paintCurves(QPainter *p);
    void paintLabels(QPainter *p);
    QColor colorOf(int channel) const;
    QPolygonF polyline(const QVector<double> &values, double top, double bottom) const;

    QPointer<GraphController> m_graph;
    int m_layers = Grid | Curves | Labels;
    QList<int> m_channels;
    int m_active = -1;
    bool m_follows = true;
    bool m_excludeOverlay = true;
    QVariantList m_colorList;
    QVector<QColor> m_colors;
    QColor m_gridColor{ Qt::white };
    QColor m_labelColor{ Qt::gray };
    QColor m_phaseColor{ Qt::lightGray };
    QFont m_font;
    bool m_animated = true;

    // Curves by key (wire channel; the phase trace under kPhaseKey)
    QHash<int, Line> m_lines;
    QList<QList<int>> m_groups;   // the lines to draw, each its channels
    QElapsedTimer m_clock;
    QTimer m_timer;

    QList<int> m_drawnChannels;
    int m_drawnLines = 0;
    bool m_drawnPhase = false;
};

#endif // BODEPLOT_H
