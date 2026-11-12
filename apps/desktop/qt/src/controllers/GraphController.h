#ifndef GRAPHCONTROLLER_H
#define GRAPHCONTROLLER_H

#include "Registrar.h"

#include <QHash>
#include <QList>
#include <QObject>
#include <QSettings>
#include <QVector>
#include <QtQml/qqmlregistration.h>
#include <array>

#include "CoreApi.h"

class DeviceModel;
class Poller;
class QQmlEngine;
class QJSEngine;

// -- WP QP3 -- The response graph's model (the Mac's BodePlotView inputs,
// GraphOptionsPanel's settings and GraphWindowController): QML sees it as the
// singleton GraphController.
//
// - Curves: every wire channel's magnitude and phase curve on the core's grid
//   (MAGNITUDE_POINTS points, 10 Hz to 20 kHz, evenly spaced in log
//   frequency), from dspi_compute_magnitude_curve / dspi_compute_phase_curve
//   over the channel's PEQ bands and, for an output, its crossover bands.
//   These are pure functions of the bands (no handle, no device), so they run
//   here on the GUI thread when a snapshot changes a channel's bands; nothing
//   of the maths lives in the app.
// - Visibility: which curves the main graph draws (the sidebar's curve pills,
//   QP1, call toggleCurve/setCurveVisible), soloed by a channel page and handed
//   back on the way to the overview, as the Mac does. The pop-out window has
//   its own set while it does not follow the selection.
// - Preferences: the Graphing settings (QSettings group "graph", the keys of
//   the previous desktop app, the Mac's defaults). The Graphing settings page
//   (QP10) binds to these properties as the graph's own setup page does.
// - Overlay slots: QML types other packages put into the graph without editing
//   its files, named with registerSlot() at static initialisation:
//       static const GraphController::Slot rta(GraphController::RtaOverlay, "RtaGraphOverlay");
//   See GraphPlot.qml for what each slot's item is given.
class GraphController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    // Selection: -1 for the overview, else the wire channel of the page shown
    Q_PROPERTY(int selectedChannel READ selectedChannel NOTIFY selectionChanged)
    // Curves and visibility
    Q_PROPERTY(int curveRevision READ curveRevision NOTIFY curvesChanged)
    Q_PROPERTY(QList<int> visibleChannels READ visibleChannels NOTIFY visibilityChanged)
    Q_PROPERTY(QList<int> popOutVisibleChannels READ popOutVisibleChannels NOTIFY visibilityChanged)
    // The channel a PEQ overlay (QP4) draws itself: left out of the curves
    // here. -1 for none.
    Q_PROPERTY(int overlayChannel READ overlayChannel WRITE setOverlayChannel NOTIFY overlayChannelChanged)
    // The pop-out window is open (the main window then hides its graph)
    Q_PROPERTY(bool poppedOut READ poppedOut WRITE setPoppedOut NOTIFY poppedOutChanged)

    // Preferences (Graphing): scale
    Q_PROPERTY(double minFreq READ minFreq WRITE setMinFreq NOTIFY preferencesChanged)
    Q_PROPERTY(double maxFreq READ maxFreq WRITE setMaxFreq NOTIFY preferencesChanged)
    Q_PROPERTY(double dbRange READ dbRange WRITE setDbRange NOTIFY preferencesChanged)
    Q_PROPERTY(double dbCenter READ dbCenter WRITE setDbCenter NOTIFY preferencesChanged)
    Q_PROPERTY(double dbTop READ dbTop NOTIFY preferencesChanged)
    Q_PROPERTY(double dbBottom READ dbBottom NOTIFY preferencesChanged)
    Q_PROPERTY(double phaseTop READ phaseTop NOTIFY preferencesChanged)
    // grid and labels
    Q_PROPERTY(bool showFreqGrid READ showFreqGrid WRITE setShowFreqGrid NOTIFY preferencesChanged)
    Q_PROPERTY(bool showFreqLabels READ showFreqLabels WRITE setShowFreqLabels NOTIFY preferencesChanged)
    Q_PROPERTY(bool showDbGrid READ showDbGrid WRITE setShowDbGrid NOTIFY preferencesChanged)
    Q_PROPERTY(bool showDbLabels READ showDbLabels WRITE setShowDbLabels NOTIFY preferencesChanged)
    Q_PROPERTY(bool freqReadout READ freqReadout WRITE setFreqReadout NOTIFY preferencesChanged)
    Q_PROPERTY(bool levelReadout READ levelReadout WRITE setLevelReadout NOTIFY preferencesChanged)
    Q_PROPERTY(double gridOpacity READ gridOpacity WRITE setGridOpacity NOTIFY preferencesChanged)
    // curves
    Q_PROPERTY(double lineWidth READ lineWidth WRITE setLineWidth NOTIFY preferencesChanged)
    Q_PROPERTY(bool showGlow READ showGlow WRITE setShowGlow NOTIFY preferencesChanged)
    Q_PROPERTY(bool showPhase READ showPhase WRITE setShowPhase NOTIFY preferencesChanged)
    Q_PROPERTY(bool phaseUnwrapped READ phaseUnwrapped WRITE setPhaseUnwrapped NOTIFY preferencesChanged)
    // the main window's graph height and the pop-out's follow switch
    Q_PROPERTY(double graphHeight READ graphHeight WRITE setGraphHeight NOTIFY preferencesChanged)
    Q_PROPERTY(bool popOutFollows READ popOutFollows WRITE setPopOutFollows NOTIFY preferencesChanged)

public:
    // The places other packages fill (GraphPlot.qml, GraphOptions.qml)
    enum SlotKind {
        RtaOverlay = 0,   // QP9: the spectrum, between the grid and the curves
        RtaOptions = 1,   // QP9: the analyser's part of the options popover
        PeqOverlay = 2,   // QP4: the on-graph editor, above the curves and labels
        PeqDock = 3,      // QP4: the band chip, docked under the main window's graph
    };
    Q_ENUM(SlotKind)
    struct Slot {
        Slot(SlotKind kind, const char *qmlType) { registerSlot(kind, qmlType); }
    };
    static void registerSlot(SlotKind kind, const char *qmlType);

    // The graph's limits, as the Mac's setup page offers them
    static constexpr double kHeightMin = 200, kHeightMax = 350;
    static constexpr double kRangeMin = 10, kRangeMax = 100;
    static constexpr double kCenterMin = -40, kCenterMax = 20;
    static constexpr double kLineMin = 1, kLineMax = 4;
    static constexpr double kGridOpacityMax = 2;

    GraphController(const AppContext &ctx, QObject *parent);
    static GraphController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<GraphController>(); }

    int selectedChannel() const { return m_selected; }
    int curveRevision() const { return m_revision; }
    QList<int> visibleChannels() const { return channelsOf(m_visible); }
    QList<int> popOutVisibleChannels() const;
    int overlayChannel() const { return m_overlayChannel; }
    void setOverlayChannel(int channel);
    bool poppedOut() const { return m_poppedOut; }
    void setPoppedOut(bool shown);

    // The curves of wire channel `channel`, empty without a device or past the
    // device's channels. Magnitudes in dB without the output's gain (see
    // outputGain), phases in degrees.
    const QVector<double> &magnitudes(int channel) const;
    const QVector<double> &phases(int channel, bool unwrapped) const;
    // An output channel's gain in dB, with a live preview winning; 0 for inputs
    double outputGain(int channel) const;
    // Wire channels: inputs 0..firstOutput-1, then the outputs
    int firstOutput() const { return m_firstOutput; }
    int channelCount() const { return m_channelCount; }
    bool connected() const { return m_channelCount > 0; }

    // QML
    Q_INVOKABLE QVariantList magnitudeCurve(int channel) const;
    Q_INVOKABLE QVariantList phaseCurve(int channel) const;
    Q_INVOKABLE bool isCurveVisible(int channel) const;
    Q_INVOKABLE bool isPopOutCurveVisible(int channel) const;
    // The sidebar's curve pills and the pop-out legend. In the pop-out, while it
    // does not follow the selection, the pop-out's own set changes.
    Q_INVOKABLE void toggleCurve(int channel);
    Q_INVOKABLE void setCurveVisible(int channel, bool visible);
    Q_INVOKABLE void togglePopOutCurve(int channel);
    // The right pane's selection: -1 for the overview. `partner` is the other
    // half of a linked input pair, shown together with it (QP2), or -1.
    Q_INVOKABLE void select(int channel, int partner = -1);
    // Every active input and enabled output (after a change of the channel set)
    Q_INVOKABLE void resetDefaultVisibility();
    // An output gain slider's value mid-drag (QP2): the curve follows it
    // before the write lands. end clears it, on release.
    Q_INVOKABLE void previewOutputGain(int output, double db);
    Q_INVOKABLE void endOutputGainPreview(int output);
    // The setup page's Reset: the scale's defaults, grids and curves untouched
    Q_INVOKABLE void resetScale();
    // Closes the pop-out window (the main window's header button)
    Q_INVOKABLE void closePopOut();
    // A slot's QML type of module UConsole, "" when no package fills it
    Q_INVOKABLE QString slotType(int kind) const;

    double minFreq() const { return m_minFreq; }
    void setMinFreq(double hz);
    double maxFreq() const { return m_maxFreq; }
    void setMaxFreq(double hz);
    double dbRange() const { return m_dbRange; }
    void setDbRange(double db);
    double dbCenter() const { return m_dbCenter; }
    void setDbCenter(double db);
    double dbTop() const { return m_dbCenter + m_dbRange / 2; }
    double dbBottom() const { return m_dbCenter - m_dbRange / 2; }
    // The phase axis follows the dB zoom: ±180° at the default 50 dB span
    double phaseTop() const { return 180.0 * m_dbRange / 50.0; }
    bool showFreqGrid() const { return m_flags.value(QStringLiteral("showFreqGrid")); }
    void setShowFreqGrid(bool on) { setFlag(QStringLiteral("showFreqGrid"), on); }
    bool showFreqLabels() const { return m_flags.value(QStringLiteral("showFreqLabels")); }
    void setShowFreqLabels(bool on) { setFlag(QStringLiteral("showFreqLabels"), on); }
    bool showDbGrid() const { return m_flags.value(QStringLiteral("showDbGrid")); }
    void setShowDbGrid(bool on) { setFlag(QStringLiteral("showDbGrid"), on); }
    bool showDbLabels() const { return m_flags.value(QStringLiteral("showDbLabels")); }
    void setShowDbLabels(bool on) { setFlag(QStringLiteral("showDbLabels"), on); }
    bool freqReadout() const { return m_flags.value(QStringLiteral("freqReadout")); }
    void setFreqReadout(bool on) { setFlag(QStringLiteral("freqReadout"), on); }
    bool levelReadout() const { return m_flags.value(QStringLiteral("levelReadout")); }
    void setLevelReadout(bool on) { setFlag(QStringLiteral("levelReadout"), on); }
    bool showGlow() const { return m_flags.value(QStringLiteral("showGlow")); }
    void setShowGlow(bool on) { setFlag(QStringLiteral("showGlow"), on); }
    bool showPhase() const { return m_flags.value(QStringLiteral("showPhase")); }
    void setShowPhase(bool on) { setFlag(QStringLiteral("showPhase"), on); }
    bool phaseUnwrapped() const { return m_flags.value(QStringLiteral("phaseUnwrapped")); }
    void setPhaseUnwrapped(bool on) { setFlag(QStringLiteral("phaseUnwrapped"), on); }
    bool popOutFollows() const { return m_flags.value(QStringLiteral("popOutFollows")); }
    void setPopOutFollows(bool on);
    double gridOpacity() const { return m_gridOpacity; }
    void setGridOpacity(double v);
    double lineWidth() const { return m_lineWidth; }
    void setLineWidth(double v);
    double graphHeight() const { return m_height; }
    void setGraphHeight(double v);

signals:
    void selectionChanged();
    void curvesChanged();
    void visibilityChanged();
    void overlayChannelChanged();
    void poppedOutChanged();
    void preferencesChanged();
    void popOutCloseRequested();

private:
    using Visibility = std::array<bool, MAX_CHANNELS>;
    struct Chain {
        std::array<FilterParams, BANDS_PER_CHANNEL + MAX_XOVER_BANDS> bands{};
        uint32_t count = 0;
        bool flat = false;
        bool operator==(const Chain &o) const;
    };

    void onSnapshot();
    void onStatus();
    Chain chainOf(const DspState &s, int channel) const;
    void applyDefaults(Visibility &v) const;
    int effectiveInputs() const;
    static QList<int> channelsOf(const Visibility &v);
    void setFlag(const QString &key, bool on);
    void store(const QString &key, const QVariant &value);

    DeviceModel *m_device;
    Poller *m_poller;
    QSettings m_settings;

    // Curves per wire channel
    std::array<Chain, MAX_CHANNELS> m_chains{};
    std::array<QVector<double>, MAX_CHANNELS> m_mag, m_phase, m_phaseUnwrapped;
    std::array<double, MAX_OUTPUTS> m_gain{};
    QHash<int, double> m_gainPreview;
    int m_firstOutput = 0;
    int m_channelCount = 0;
    int m_outputCount = 0;
    uint64_t m_generation = 0;
    std::array<bool, MAX_OUTPUTS> m_enabled{};
    int m_revision = 0;

    // Visibility and selection
    Visibility m_visible{};
    Visibility m_popOut{};
    bool m_popOutSet = false;
    Visibility m_savedOverview{};
    bool m_haveSaved = false;
    int m_selected = -1;
    int m_overlayChannel = -1;
    bool m_poppedOut = false;
    int m_inputsShown = 0;

    // Preferences
    double m_minFreq, m_maxFreq, m_dbRange, m_dbCenter, m_gridOpacity, m_lineWidth, m_height;
    QHash<QString, bool> m_flags;
};

#endif // GRAPHCONTROLLER_H
