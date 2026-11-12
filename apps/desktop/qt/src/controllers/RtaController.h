#ifndef RTACONTROLLER_H
#define RTACONTROLLER_H

#include <QHash>
#include <QList>
#include <QObject>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <memory>
#include <mutex>

#include "CoreApi.h"
#include "Registrar.h"

class Core;
class PrefsMap;
class QQmlEngine;
class QJSEngine;

// -- WP QP9 -- The spectrum analyser (the Mac's SpectrumAnalyser, the
// selections of SpectrumAnalyserView and the views' registrations): one
// request to the core, the device's capabilities, the latest picture and who
// drives the analyser.
//
// - Views: the graph overlay (main window, pop-out), the bar strip and the
//   analyser window register with setView() while they draw. The core holds
//   one request, built here from the page's own selection: its mask while any
//   view watches (0 otherwise, which stops the analyser on the device), bins
//   while a curve view watches, the transform size, averaging and decay from
//   the Spectrum Analyser settings. Nothing is asked while nothing watches.
// - Selections: the dashboard's is stored (spectrum/dashboardSource and the
//   other side); a channel page starts on its own channel and is not stored.
// - Following: when another console drives the analyser, every view draws the
//   device's selection (`drawnTap`, `drawnChannels`) and keeps asking for its
//   own; a choice made in the options, or Take control, takes it back.
// - The core's analyser thread wakes this object; the picture is copied there
//   (the core allows its reads from that callback) and handed to the GUI
//   thread, which never takes the core's lock.
// - dspi_rta_start runs after every connect and after a Link reconnect.
//
// The renderers (RtaSurface, RtaBars) read snapshot() and the scale here.
class RtaController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The device and its analyser
    Q_PROPERTY(bool ready READ ready NOTIFY analyserChanged)
    Q_PROPERTY(bool available READ available NOTIFY analyserChanged)
    Q_PROPERTY(int largestOrder READ largestOrder NOTIFY analyserChanged)
    // The page: the dashboard, or a channel page
    Q_PROPERTY(bool onDashboard READ onDashboard NOTIFY selectionChanged)
    Q_PROPERTY(QString pageTitle READ pageTitle NOTIFY selectionChanged)
    // The page's own selection and what the views draw (the device's while following)
    Q_PROPERTY(int ownTap READ ownTap NOTIFY selectionChanged)
    Q_PROPERTY(QList<int> ownChannels READ ownChannels NOTIFY selectionChanged)
    Q_PROPERTY(int drawnTap READ drawnTap NOTIFY selectionChanged)
    Q_PROPERTY(QList<int> drawnChannels READ drawnChannels NOTIFY selectionChanged)
    // The page shows the spectrum on the graph, as bars
    Q_PROPERTY(bool showGraph READ showGraph WRITE setShowGraph NOTIFY selectionChanged)
    Q_PROPERTY(bool showBars READ showBars WRITE setShowBars NOTIFY selectionChanged)
    // Ownership
    Q_PROPERTY(bool following READ following NOTIFY ownershipChanged)
    // Telemetry (the window's status bar)
    Q_PROPERTY(bool analysing READ analysing NOTIFY readingsChanged)
    Q_PROPERTY(bool refused READ refused NOTIFY readingsChanged)
    Q_PROPERTY(QStringList statusParts READ statusParts NOTIFY readingsChanged)
    Q_PROPERTY(QString bassText READ bassText NOTIFY readingsChanged)
    Q_PROPERTY(QString lowBandText READ lowBandText NOTIFY readingsChanged)
    Q_PROPERTY(QString rangeText READ rangeText NOTIFY readingsChanged)
    // Rises with every new picture (the renderers repaint on it)
    Q_PROPERTY(int pictureRevision READ pictureRevision NOTIFY pictureChanged)
    // The request asks for something
    Q_PROPERTY(bool watching READ watching NOTIFY requestChanged)
    // The strip's and window's layout and the views' scale (spectrum/* settings)
    Q_PROPERTY(PrefsMap *prefs READ prefs CONSTANT)
    Q_PROPERTY(double floorDb READ floorDb NOTIFY scaleChanged)
    Q_PROPERTY(double ceilingDb READ ceilingDb NOTIFY scaleChanged)
    Q_PROPERTY(bool showPeakHold READ showPeakHold NOTIFY scaleChanged)
    Q_PROPERTY(double graphOpacity READ graphOpacity NOTIFY scaleChanged)
    Q_PROPERTY(double smoothing READ smoothing NOTIFY scaleChanged)

public:
    // What a view draws: curves (with the bins when one channel) or bars
    enum ViewKind { Curves = 0, Bars = 1 };
    Q_ENUM(ViewKind)
    static constexpr int kBarHeightMin = 64, kBarHeightMax = 240;
    static constexpr int kMaxColumns = 4;
    // The selection's smoothing amount when the preference is on (the Mac's 0.35)
    static constexpr double kSmoothingAmount = 0.35;

    struct Selection {
        int tap = RTA_TAP_OUTPUT;
        QList<int> channels;
        bool operator==(const Selection &o) const { return key() == o.key(); }
        uint16_t mask() const;
        // "in:0,2", "out:" (nothing chosen)
        QString key() const;
        // false for anything else than in:/out: with channels 0..15
        static bool parse(const QString &key, Selection *out);
    };

    RtaController(const AppContext &ctx, QObject *parent = nullptr);
    static RtaController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<RtaController>(); }

    bool ready() const;
    bool available() const;
    int largestOrder() const;
    const RtaCapsInfo &capabilities() const { return m_caps; }
    // The latest picture (GUI thread)
    const RtaSnapshot &snapshot() const { return *m_snap; }
    int pictureRevision() const;

    bool onDashboard() const;
    QString pageTitle() const;
    int ownTap() const;
    QList<int> ownChannels() const;
    int drawnTap() const;
    QList<int> drawnChannels() const;
    Selection own() const;
    Selection drawn() const;
    bool showGraph() const;
    void setShowGraph(bool on);
    bool showBars() const;
    void setShowBars(bool on);
    bool following() const;

    bool analysing() const;
    bool refused() const;
    QStringList statusParts() const;
    QString bassText() const;
    QString lowBandText() const;
    QString rangeText() const;
    bool watching() const;

    PrefsMap *prefs() const { return m_prefs; }
    double floorDb() const;
    double ceilingDb() const;
    bool showPeakHold() const;
    double graphOpacity() const;
    double smoothing() const;
    // The time constant the renderers glide with (0: draw the device's numbers)
    double smoothingTau() const;
    // Band b is drawn: resolved at this rate and transform size
    bool bandVisible(int band) const;
    // Bands the picture can have
    int bandCount() const;

    // The channels the page can choose at `tap`: {channel, name, wire, kind, number, pdm}
    Q_INVOKABLE QVariantList channels(int tap) const;
    // One channel at a tap: {name, wire, kind, number, pdm}
    Q_INVOKABLE QVariantMap channelInfo(int tap, int channel) const;
    // The options: a chip, a side, Clear (each takes the analyser back while following)
    Q_INVOKABLE void toggleChannel(int channel);
    Q_INVOKABLE void switchSide(int tap);
    Q_INVOKABLE void clearSelection();
    Q_INVOKABLE void takeControl();
    // A view draws (`active`) or not; `id` names it ("graph", "graph.popOut",
    // "strip", "window.curves", "window.bars")
    Q_INVOKABLE void setView(const QString &id, int kind, bool active);
    Q_INVOKABLE bool viewActive(const QString &id) const { return m_views.contains(id); }
    // "1k", "200"
    Q_INVOKABLE static QString shortHz(double hz);
    // The bar layout (strip and window): columns for `count` channels, a
    // cell's height, and the stored height after dragging the strip's edge
    // `dy` down from `startHeight`
    Q_INVOKABLE int barColumnsFor(int count) const;
    Q_INVOKABLE int barCellHeight(int count) const;
    Q_INVOKABLE int draggedBarHeight(double startHeight, double dy, int count) const;

    // The request as last sent (tests)
    const RtaRequest &request() const { return m_request; }
    // How many requests were sent (tests)
    int requestsSent() const;

signals:
    void analyserChanged();
    void selectionChanged();
    void ownershipChanged();
    void readingsChanged();
    void pictureChanged();
    void requestChanged();
    void scaleChanged();

private:
    struct Shared;
    // Unsets the analyser's callback when the controller goes (after
    // Core::shutdown there is nothing left to unset)
    struct CallbackRelease {
        Core *core = nullptr;
        CallbackRelease() = default;
        CallbackRelease(const CallbackRelease &) = delete;
        CallbackRelease &operator=(const CallbackRelease &) = delete;
        ~CallbackRelease();
    };
    static void thunk(void *self);
    Q_INVOKABLE void onPicture();
    // Stores a spectrum/* key of this package's and updates the map QML binds to
    void setPref(const char *key, const QVariant &value);
    void onSnapshot();
    void onPageChanged();
    void connectLater();
    void setOwn(const Selection &s, bool takeBack);
    void storeDashboard(const Selection &s);
    Selection restricted(const Selection &s) const;
    Selection dashboardSelection() const;
    QList<int> available(int tap) const;
    void push();
    void startAnalyser();

    AppContext m_ctx;
    PrefsMap *m_prefs;
    // The settings page's map (fftOrder, avgMs, ...), found once the controllers exist
    PrefsMap *m_settings = nullptr;
    std::shared_ptr<Shared> m_shared;
    std::shared_ptr<RtaSnapshot> m_snap;
    // After m_shared: released first
    CallbackRelease m_release;
    RtaCapsInfo m_caps{};
    RtaOwnership m_owner{};
    uint32_t m_version = 0;
    uint32_t m_telemetry = 0;
    int m_pictureRevision = 0;
    uint64_t m_generation = 0;
    // -1 on the dashboard, else the wire channel of the page
    int m_page = -1;
    Selection m_pageSelection;
    Selection m_pageOtherSide;
    bool m_pageOtherKnown = false;
    QHash<QString, int> m_views;
    RtaRequest m_request{};
    bool m_watching = false;
    bool m_sentOnce = false;
    int m_requestsSent = 0;
};

#endif // RTACONTROLLER_H
