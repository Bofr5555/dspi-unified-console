#ifndef PEQOVERLAY_H
#define PEQOVERLAY_H

#include <QColor>
#include <QElapsedTimer>
#include <QHash>
#include <QList>
#include <QPointer>
#include <QPolygonF>
#include <QQuickPaintedItem>
#include <QSet>
#include <QTimer>
#include <QVariantMap>
#include <QVector>
#include <QtQml/qqmlregistration.h>
#include <array>
#include <functional>
#include <optional>

#include "CoreApi.h"

class GraphController;
class PeqEditorController;

// -- WP QP4 -- On-graph PEQ band editing (the Mac's PeqGraphEditorView and
// its renderer): the edited channel's curve, each band's lobe and dot, the
// ghost dot and the marquee, and every pointer and key gesture. The band chip,
// the shape card, the readouts and the menus are QML (qml/graph), driven by
// this item's properties and invokables.
//
// Pointer (the Mac's Cmd is Ctrl, its Option is Alt):
//   hover a band's fill   lights it; only its dot takes clicks
//   hover empty graph     a faint dot where a double-click makes a bell, with
//                         frequency and level readouts
//   double-click empty    a bell there;  Ctrl-click: the shape card
//   click empty           deselect;  drag the curve: pull a bell (a shelf near
//                         either end) out of it;  drag empty: marquee
//   click a dot           select; Ctrl toggles, Shift takes a range
//   drag a dot            frequency and gain (Q for second-order cuts;
//                         frequency only for dots at a fixed level)
//   Ctrl-drag             Q of the selection;  Shift fine;  Alt one axis
//   Alt-click             bypass;  double-click a dot: type its values
//   wheel over a band     Q; Ctrl-wheel gain; Shift fine
//   wheel at the left edge (no band)   the dB zoom
//   right-click           the band menu on a dot, the graph menu elsewhere
// Keys: Delete, Escape, Ctrl+A, arrows (Alt-arrows Q), Tab, Enter (type the
// values), Shift+F10 or the menu key (the menus).
//
// The response is the core's (dspi_compute_response). Device writes go out at
// up to 30 a second during a gesture, and the band list follows through
// PeqEditorController's live values.
class PeqOverlay : public QQuickPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(GraphController *graph READ graph WRITE setGraph NOTIFY graphChanged)
    // The page's channel when this graph follows it, else -1
    Q_PROPERTY(int pageChannel READ pageChannel WRITE setPageChannel NOTIFY pageChannelChanged)
    Q_PROPERTY(QColor curveColor READ curveColor WRITE setCurveColor NOTIFY curveColorChanged)
    // A colour per band (PeqSelection.qml's palette)
    Q_PROPERTY(QVariantList bandColors READ bandColors WRITE setBandColors NOTIFY bandColorsChanged)
    // The plot's background (a selected dot's centre) and the ink of the ghost
    // dot and the marquee
    Q_PROPERTY(QColor backdrop READ backdrop WRITE setBackdrop NOTIFY backdropChanged)
    Q_PROPERTY(QColor ink READ ink WRITE setInk NOTIFY inkChanged)

    // State for QML (all NOTIFY viewChanged)
    Q_PROPERTY(bool editing READ editing NOTIFY viewChanged)
    Q_PROPERTY(int editedChannel READ editedChannel NOTIFY viewChanged)
    Q_PROPERTY(QList<int> selectedBands READ selectedBands NOTIFY viewChanged)
    Q_PROPERTY(int pointedBand READ pointedBand NOTIFY viewChanged)
    Q_PROPERTY(bool ghostShown READ ghostShown NOTIFY viewChanged)
    Q_PROPERTY(bool marqueeShown READ marqueeShown NOTIFY viewChanged)
    Q_PROPERTY(bool animating READ animating NOTIFY viewChanged)
    // The band the bar under the graph shows (PeqBandBar.qml; -1 none) and
    // what it shows (-- WP QPI2 -- docked, the owner's choice over the
    // Mac's chip beside the dot)
    Q_PROPERTY(int chipBand READ chipBand NOTIFY viewChanged)
    Q_PROPERTY(QVariantMap chip READ chip NOTIFY viewChanged)
    Q_PROPERTY(bool shapePage READ shapePage NOTIFY viewChanged)
    // The Ctrl-click card
    Q_PROPERTY(bool cardOpen READ cardOpen NOTIFY viewChanged)
    Q_PROPERTY(double cardX READ cardX NOTIFY viewChanged)
    Q_PROPERTY(double cardY READ cardY NOTIFY viewChanged)
    // The shapes this firmware offers: [{shape, title, orders: [1, 2]}]
    Q_PROPERTY(QVariantList shapes READ shapes NOTIFY viewChanged)
    // Readouts on empty graph: frequency at the bottom, level at the left
    Q_PROPERTY(QString axisText READ axisText NOTIFY viewChanged)
    Q_PROPERTY(double axisX READ axisX NOTIFY viewChanged)
    Q_PROPERTY(QString levelText READ levelText NOTIFY viewChanged)
    Q_PROPERTY(double levelY READ levelY NOTIFY viewChanged)

public:
    // The Ctrl-click card's metrics (the size of the Mac's PeqBandHUD)
    static constexpr double kChipWidth = 110;
    static constexpr double kChipFirstRow = 28;
    static constexpr double kChipRow = 15;
    static constexpr double kChipBottom = 5;

    explicit PeqOverlay(QQuickItem *parentItem = nullptr);
    ~PeqOverlay() override;

    GraphController *graph() const { return m_graph; }
    void setGraph(GraphController *g);
    int pageChannel() const { return m_pageChannel; }
    void setPageChannel(int ch);
    QColor curveColor() const { return m_curveColor; }
    void setCurveColor(const QColor &c);
    QVariantList bandColors() const { return m_bandColorList; }
    void setBandColors(const QVariantList &colors);
    QColor backdrop() const { return m_backdrop; }
    void setBackdrop(const QColor &c);
    QColor ink() const { return m_ink; }
    void setInk(const QColor &c);

    bool editing() const { return m_channel >= 0; }
    int editedChannel() const { return m_channel; }
    QList<int> selectedBands() const;
    int pointedBand() const { return m_hovered; }
    bool ghostShown() const { return m_ghost.has_value(); }
    bool marqueeShown() const { return m_gesture.kind == Gesture::Marquee; }
    bool animating() const { return m_frame.isActive(); }
    int chipBand() const { return m_chipShown ? m_chipBand : -1; }
    QVariantMap chip() const { return m_chipInfo; }
    bool shapePage() const { return m_shapePage; }
    bool cardOpen() const { return m_cardPoint.has_value(); }
    double cardX() const { return m_cardRect.x(); }
    double cardY() const { return m_cardRect.y(); }
    QVariantList shapes() const;
    QString axisText() const { return m_axisText; }
    double axisX() const { return m_axisX; }
    QString levelText() const { return m_levelText; }
    double levelY() const { return m_levelY; }

    // The band bar (PeqBandBar.qml)
    Q_INVOKABLE void chipBypass();
    Q_INVOKABLE void chipInvert(); // -- WP QPI2 --
    Q_INVOKABLE void chipDelete(); // -- WP QPI2 --
    Q_INVOKABLE void chipToggleShapes();
    Q_INVOKABLE void chipPick(int shape, int order);
    // A typed value; false when it did not parse
    Q_INVOKABLE bool chipText(int field, const QString &text);
    // A value dragged: phase 0 began, 1 changed (delta in points), 2 ended
    Q_INVOKABLE void chipAdjust(int field, double delta, int phase);
    Q_INVOKABLE void chipScroll(int field, double delta, bool fine);
    Q_INVOKABLE void chipEditing(bool editing);
    Q_INVOKABLE void chipHovered(bool inside);
    // The text a field starts with when typing begins
    Q_INVOKABLE QString chipEditText(int field) const;
    // A scroll on the chip that a graph wheel gesture should keep
    Q_INVOKABLE bool chipForwardsScroll() const { return graphWheelActive(); }
    // The chooser's slope buttons
    Q_INVOKABLE QString orderLabel(int shape, int order) const;
    Q_INVOKABLE QString orderHelp(int shape, int order) const;
    // The card
    Q_INVOKABLE void cardPick(int shape, int order);
    Q_INVOKABLE void closeCard();
    // The menus (PeqGraphOverlay.qml): what the last menuRequested offers,
    // and its rows' actions
    Q_INVOKABLE QVariantMap menuModel() const;
    Q_INVOKABLE void menuAction(const QString &action, int argument = 0);

    // The pointer over the plot, from a HoverHandler (PeqGraphOverlay.qml)
    Q_INVOKABLE void hoverAt(const QPointF &p);
    Q_INVOKABLE void hoverLeft();

    // For tests: a band's dot in item coordinates, every animation finished
    Q_INVOKABLE QPointF dotPoint(int band) const;
    Q_INVOKABLE void settle();
    Q_INVOKABLE QVariantMap shownBand(int band) const;
    Q_INVOKABLE double curveDbAt(double hz) const;

    // Runs in the scene graph's sync: reads, never writes
    void paint(QPainter *canvas) override;

signals:
    void graphChanged();
    void pageChannelChanged();
    void curveColorChanged();
    void bandColorsChanged();
    void backdropChanged();
    void inkChanged();
    void viewChanged();
    // Typing begins in the chip's `field`
    void editRequested(int field);
    // The band or graph menu, at a point in item coordinates (menuModel())
    void menuRequested(double x, double y);
    // An action had nothing to act on (the Mac beeps)
    void refused();

protected:
    bool event(QEvent *ev) override;
    void geometryChange(const QRectF &now, const QRectF &before) override;
    void mousePressEvent(QMouseEvent *ev) override;
    void mouseMoveEvent(QMouseEvent *ev) override;
    void mouseReleaseEvent(QMouseEvent *ev) override;
    void mouseDoubleClickEvent(QMouseEvent *ev) override;
    void wheelEvent(QWheelEvent *ev) override;
    void keyPressEvent(QKeyEvent *ev) override;
    void itemChange(ItemChange what, const ItemChangeData &data) override;

private:
    struct Gesture {
        enum Kind { Idle, Press, Background, Drag, Marquee } kind = Idle;
        int band = -1;
        QPointF at;
        Qt::KeyboardModifiers mods;
        bool onCurve = false;
        // Drag: the bands taken and how they started
        int grabbed = -1;
        QHash<int, FilterParams> start;
        QPointF last, offset;
        bool qMode = false;
        int axis = 0;   // Alt's lock: 0 free, 1 horizontal, 2 vertical
        bool pending = false;
        QPointF gathered;
        // Marquee
        QPointF to;
        QSet<int> base;
    };
    struct Emphasis {
        float hover = 0, select = 0, hoverTarget = 0, selectTarget = 0;
        bool settled() const { return hover == hoverTarget && select == selectTarget; }
    };
    // What paint() draws, made on the GUI thread
    struct Picture {
        struct Lobe {
            QPolygonF fill, line;
            QColor color;
            double fillOpacity = 0, lineOpacity = 0, edgeY = 0, zeroY = 0;
        };
        struct Dot {
            QPointF at;
            double radius = 5, alpha = 1, select = 0, order = 0;
            QColor color;
        };
        QList<Lobe> lobes;
        QPolygonF curve;
        QColor curveColor;
        double lineWidth = 1.5;
        bool glow = false;
        QList<Dot> dots;
        QRectF marquee;
    };

    // Configuration (the Mac's PeqGraphEditorConfig), read from the graph,
    // the channel controller and the device
    void reconfigure();
    void configure();
    void writing(const std::function<void()> &write);
    void readBands(QVector<FilterParams> &bands, QVector<FilterParams> &statics) const;
    void resetInteraction();
    bool takesKey(const QKeyEvent *e) const;
    void updateOwnership();

    // Mapping
    double xOf(double hz) const;
    double freqAt(double x) const;
    double yOf(double db) const;
    double dbAt(double y) const;
    double dbPerPoint() const;
    double freqLo() const;
    double freqHi() const;

    // Bands
    int bandCount() const { return qMin(int(m_shown.size()), int(BANDS_PER_CHANNEL)); }
    bool isBand(int b) const;
    QList<int> graphBands() const;
    QPointF dotOf(const FilterParams &p) const;
    double combinedDb(double hz) const;
    int dotUnder(const QPointF &p) const;
    int fillUnder(const QPointF &p) const;
    int bandUnder(const QPointF &p) const;
    bool onCurveAt(const QPointF &p) const;
    int freeSlot() const;
    QList<int> frequencyOrder(QList<int> bands) const;
    FilterParams current(int b) const;
    FilterParams creation(const QPointF &p, bool fromCurve) const;
    std::optional<FilterParams> shaped(int shape, int order, const QPointF &p) const;
    QSet<int> targets(int b) const;
    int selectedChipBand() const;
    const QSet<int> &selection() const;

    // Drawing and animation
    void invalidate();
    void updateEmphasisTargets();
    void startFrames();
    void tick();
    bool advance(double now);
    void rebuild();
    void startBandAnimation();
    double now() const;

    // The editing pipeline
    void setLive(const QHash<int, FilterParams> &changes);
    void sendToDevice();
    void commitLive();
    // -- WP QTGAP1 -- take or let go of DeviceModel's drag mark: a pointer
    // drag of a dot or a chip field is one undo step; wheel and key edits are not
    void holdDrag(bool on);
    bool m_holdsDrag = false;
    void setLiveThenCommit(const QHash<int, FilterParams> &changes);
    void commitNow(const QHash<int, FilterParams> &changes);
    void setSelection(const QSet<int> &bands);
    void setHovered(int band);
    void scheduleReveal();

    // Operations
    int createBand(const FilterParams &p, bool select = true);
    void deleteBands(const QSet<int> &bands);
    void toggleBypass(const QSet<int> &bands);
    void setShape(const QSet<int> &bands, int shape, int order);
    void invertGain(const QSet<int> &bands);

    // Pointer
    void pointerMoved(const QPointF &p);
    void updateHover(const QPointF &p);
    void press(const QPointF &p, Qt::KeyboardModifiers mods, bool doubleClick);
    void beginDrag(int grabbed, const QSet<int> &bands, const QPointF &at, bool qMode);
    void continueDrag(const QPointF &p, Qt::KeyboardModifiers modifiers);
    QHash<int, FilterParams> dragResult(const Gesture &g) const;
    void updateMarquee();
    bool handleWheel(const QPointF &p, double delta, Qt::KeyboardModifiers mods, bool begins);
    void openMenu(const QPointF &p, int band);

    // Keys
    void stepSelection(bool backwards);
    void nudge(int key, Qt::KeyboardModifiers modifiers);

    // Chip and card
    void showChip(int b);
    void refreshChip();
    QRectF panelFrame(const QPointF &point, const QSizeF &size, bool boost) const;
    void pinChipToSelection();
    void scheduleChipHide();
    void hideChip();
    void closeShapePage();
    std::optional<FilterParams> adjusted(const FilterParams &p, int field, double delta) const;
    bool graphWheelActive() const;
    bool chipWheelActive() const;
    void openCard(const QPointF &p);
    void showReadouts(const QPointF &p);
    void hideReadouts();

    QPointer<GraphController> m_graph;
    PeqEditorController *m_editor = nullptr;
    QPointer<QQuickWindow> m_window; // -- WP QPI2 -- the window whose visibility we follow
    int m_pageChannel = -1;
    QColor m_curveColor{ Qt::white };
    QVariantList m_bandColorList;
    QVector<QColor> m_bandColors;
    QColor m_backdrop{ Qt::black };
    QColor m_ink{ Qt::white };
    int m_writing = 0;
    bool m_inReconfigure = false;
    bool m_reconfigurePending = false;

    // Configuration
    int m_channel = -1;
    QVector<FilterParams> m_statics;
    double m_offsetDb = 0;
    bool m_flat = false;
    QSet<int> m_available;
    bool m_bypassSupported = false;

    // Bands: committed mirrors the device (staged edits included), live holds
    // edits not yet committed, shown is what is drawn
    QVector<FilterParams> m_committed, m_shown;
    QHash<int, FilterParams> m_live;
    std::optional<QVector<FilterParams>> m_animFrom;
    double m_animStart = 0;

    int m_rangeAnchor = -1;
    int m_hovered = -1;
    int m_rowHovered = -1;
    std::optional<FilterParams> m_ghost, m_ghostShown;
    float m_ghostAmount = 0;
    std::optional<QPointF> m_pointer;
    Gesture m_gesture;
    QVector<Emphasis> m_emphasis;
    double m_lastFrame = 0;

    // Chip
    int m_chipBand = -1;
    bool m_chipShown = false;
    bool m_chipEditing = false;
    bool m_shapePage = false;
    QVariantMap m_chipInfo;
    std::optional<FilterParams> m_chipAdjustStart;
    bool m_overChip = false;
    // Card
    std::optional<QPointF> m_cardPoint;
    QRectF m_cardRect;
    // Readouts
    QString m_axisText, m_levelText;
    double m_axisX = 0, m_levelY = 0;
    // Menus
    QSet<int> m_menuTargets;
    int m_menuBand = -1;

    // Wheel sessions
    int m_wheelTarget = -1;
    double m_wheelTime = -10;
    int m_chipWheelField = -1;
    double m_chipWheelTime = -10;

    QTimer m_frame, m_chipHide, m_reveal, m_device, m_commit;
    QSet<int> m_deviceDirty;
    QElapsedTimer m_clock;
    Picture m_picture;
    // What the lines were worked out for
    using Scale = std::array<double, 8>;
    Scale m_scaleKey{};
    QHash<int, QPair<FilterParams, QPolygonF>> m_lineCache;
    QVector<FilterParams> m_curveKey;
    QPolygonF m_curve;
};

// The small glyph of a shape (the Mac's PeqShapeGlyph), 18 by 12 points
class PeqShapeGlyph : public QQuickPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(int shape READ shape WRITE setShape NOTIFY glyphChanged)
    Q_PROPERTY(QColor ink READ ink WRITE setInk NOTIFY glyphChanged)
public:
    explicit PeqShapeGlyph(QQuickItem *parentItem = nullptr);
    int shape() const { return m_shape; }
    void setShape(int s);
    QColor ink() const { return m_ink; }
    void setInk(const QColor &value);
    void paint(QPainter *p) override;
signals:
    void glyphChanged();

private:
    int m_shape = 0;
    QColor m_ink{ Qt::white };
};

#endif // PEQOVERLAY_H
