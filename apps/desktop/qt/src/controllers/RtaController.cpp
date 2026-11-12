// -- WP QP9 -- The spectrum analyser's controller (RtaController.h).
#include "RtaController.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "LinkController.h"
#include "PrefsMap.h"
#include "SettingsController.h"

#include <QCoreApplication>
#include <QSettings>
#include <QStringList>
#include <algorithm>
#include <cmath>

static const Registrar::Controller<RtaController> rtaController;
static const Registrar::Window spectrumWindow("tools.spectrum", "SpectrumWindow");
// The response graph's slots (GraphController::Slot)
static const GraphController::Slot rtaOverlaySlot(GraphController::RtaOverlay, "RtaGraphOverlay");
static const GraphController::Slot rtaOptionsSlot(GraphController::RtaOptions, "RtaGraphOptions");

namespace {

QString tr(const char *text) { return QCoreApplication::translate("RtaController", text); }

// The keys this package adds to spectrum/* (the others are the settings page's)
const char *const kDashboardSource = "dashboardSource";
const char *const kDashboardOtherSide = "dashboardOtherSide";
const char *const kDashboardShowGraph = "dashboardShowGraph";
const char *const kDashboardShowBars = "dashboardShowBars";
const char *const kChannelShowGraph = "channelShowGraph";
const char *const kChannelShowBars = "channelShowBars";
const char *const kChannelPagesShowSpectrum = "channelPagesShowSpectrum";

} // namespace

// What the analyser thread hands over: the latest picture, caps and ownership
struct RtaController::Shared {
    std::mutex lock;
    std::shared_ptr<RtaSnapshot> snap = std::make_shared<RtaSnapshot>();
    RtaCapsInfo caps{};
    RtaOwnership owner{};
    std::atomic<bool> queued{ false };
    QObject *target = nullptr;
    FfiCore *core = nullptr;
};

// ---- Selection ----

uint16_t RtaController::Selection::mask() const {
    uint16_t m = 0;
    for (int c : channels)
        if (c >= 0 && c < 16) m |= uint16_t(1u << c);
    return m;
}

QString RtaController::Selection::key() const {
    QStringList parts;
    for (int c : channels) parts.append(QString::number(c));
    return (tap == RTA_TAP_INPUT ? QStringLiteral("in:") : QStringLiteral("out:")) + parts.join(QLatin1Char(','));
}

bool RtaController::Selection::parse(const QString &key, Selection *out) {
    Selection s;
    QString rest;
    if (key.startsWith(QLatin1String("in:"))) {
        s.tap = RTA_TAP_INPUT;
        rest = key.mid(3);
    } else if (key.startsWith(QLatin1String("out:"))) {
        s.tap = RTA_TAP_OUTPUT;
        rest = key.mid(4);
    } else {
        return false;
    }
    for (const QString &part : rest.split(QLatin1Char(','), Qt::SkipEmptyParts)) {
        bool ok = false;
        const int c = part.trimmed().toInt(&ok);
        if (!ok || c < 0 || c > 15) return false;
        if (!s.channels.contains(c)) s.channels.append(c);
    }
    std::sort(s.channels.begin(), s.channels.end());
    if (out) *out = s;
    return true;
}

// ---- Life ----

RtaController::RtaController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_prefs(new PrefsMap(QStringLiteral("spectrum"), this)),
      m_shared(std::make_shared<Shared>()), m_snap(std::make_shared<RtaSnapshot>()) {
    m_prefs->declare(QLatin1String(kDashboardSource), QString());
    m_prefs->declare(QLatin1String(kDashboardOtherSide), QString());
    m_prefs->declare(QLatin1String(kDashboardShowGraph), true);
    m_prefs->declare(QLatin1String(kDashboardShowBars), false);
    m_prefs->declare(QLatin1String(kChannelPagesShowSpectrum), true);
    m_prefs->declare(QLatin1String(kChannelShowGraph), true);
    m_prefs->declare(QLatin1String(kChannelShowBars), false);
    m_prefs->declare(QStringLiteral("barColumns"), 2);
    m_prefs->declare(QStringLiteral("barHeight"), 96);
    connect(m_prefs, &PrefsMap::valueChanged, this, [this](const QString &key) {
        if (key.startsWith(QLatin1String("dashboard")) || key.startsWith(QLatin1String("channel"))) {
            emit selectionChanged();
            push();
        }
    });
    m_snap->first_band = 0xFF;

    m_shared->target = this;
    m_shared->core = ctx.core->handle();
    Shared *shared = m_shared.get();
    ctx.core->worker()->post([shared](FfiCore *c) { dspi_rta_set_callback(c, &RtaController::thunk, shared); });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &RtaController::onSnapshot);
    // The other controllers exist once every constructor has run
    QMetaObject::invokeMethod(this, &RtaController::connectLater, Qt::QueuedConnection);
}

RtaController::~RtaController() {
    // After Core::shutdown the handle is gone and the core has dropped the callback
    if (FfiCore *c = m_ctx.core->handle()) dspi_rta_set_callback(c, nullptr, nullptr);
}

void RtaController::connectLater() {
    if (auto *graph = Registrar::instance<GraphController>()) {
        connect(graph, &GraphController::selectionChanged, this, &RtaController::onPageChanged);
        m_page = graph->selectedChannel();
    }
    if (auto *settings = Registrar::instance<SettingsController>()) {
        m_settings = settings->spectrum();
        connect(m_settings, &PrefsMap::valueChanged, this, [this](const QString &key) {
            emit scaleChanged();
            if (key == QLatin1String("fftOrder") || key == QLatin1String("avgMs") || key == QLatin1String("peakDecay")) push();
        });
    }
    // After a lost Link session is back the analyser's worker went with the
    // old connection: start it again (LinkController does it too while it
    // knows the analyser is wanted). The core keeps the request.
    if (auto *link = Registrar::instance<LinkController>())
        connect(link, &LinkController::reconnected, this, &RtaController::startAnalyser);
    emit scaleChanged();
    emit selectionChanged();
    push();
}

// The analyser's thread (or the one closing the device): copy, then schedule
void RtaController::thunk(void *user) {
    auto *shared = static_cast<Shared *>(user);
    if (!shared || !shared->core) return;
    auto snap = std::make_shared<RtaSnapshot>();
    RtaCapsInfo caps{};
    RtaOwnership owner{};
    dspi_rta_get_snapshot(shared->core, snap.get());
    dspi_rta_get_caps(shared->core, &caps);
    dspi_rta_get_ownership(shared->core, &owner);
    {
        std::lock_guard<std::mutex> hold(shared->lock);
        shared->snap = std::move(snap);
        shared->caps = caps;
        shared->owner = owner;
    }
    if (!shared->queued.exchange(true))
        QMetaObject::invokeMethod(shared->target, "onPicture", Qt::QueuedConnection);
}

void RtaController::onPicture() {
    m_shared->queued.store(false);
    std::shared_ptr<RtaSnapshot> snap;
    RtaCapsInfo caps{};
    RtaOwnership owner{};
    {
        std::lock_guard<std::mutex> hold(m_shared->lock);
        snap = m_shared->snap;
        caps = m_shared->caps;
        owner = m_shared->owner;
    }
    if (!snap) return;
    const bool capsNew = caps.supported != m_caps.supported || caps.max_bands != m_caps.max_bands
        || caps.order_max != m_caps.order_max || caps.input_channels != m_caps.input_channels
        || caps.output_channels != m_caps.output_channels
        || !std::equal(std::begin(caps.centres), std::end(caps.centres), std::begin(m_caps.centres));
    const bool ownerNew = owner.following != m_owner.following || owner.tap != m_owner.tap || owner.mask != m_owner.mask;
    const bool pictureNew = snap->version != m_version || snap->tap != m_snap->tap || snap->count != m_snap->count;
    const bool telemetryNew = snap->telemetry_version != m_telemetry || snap->running != m_snap->running
        || snap->config_rejected != m_snap->config_rejected;
    m_snap = snap;
    m_caps = caps;
    m_owner = owner;
    m_version = snap->version;
    m_telemetry = snap->telemetry_version;
    if (capsNew) {
        emit capsChanged();
        emit selectionChanged();   // the channels available follow the caps
        push();
    }
    if (ownerNew) {
        emit ownershipChanged();
        emit selectionChanged();
    }
    if (pictureNew || capsNew) {
        ++m_pictureRevision;
        emit pictureChanged();
    }
    if (telemetryNew || capsNew) emit telemetryChanged();
}

void RtaController::onSnapshot() {
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    if (!snap.connected) {
        if (m_generation != 0) {
            m_generation = 0;
            emit capsChanged();
            emit selectionChanged();
        }
        return;
    }
    if (snap.generation != m_generation) {
        m_generation = snap.generation;
        // The core keeps the request across connections; a new worker is needed
        startAnalyser();
        push();
    }
    // Names, enabled outputs and the input count can change with any snapshot
    emit selectionChanged();
}

void RtaController::startAnalyser() {
    // A no-op while running for this connection or on a read-only one
    m_ctx.core->worker()->post([](FfiCore *c) { dspi_rta_start(c); });
}

void RtaController::onPageChanged() {
    auto *graph = Registrar::instance<GraphController>();
    const int page = graph ? graph->selectedChannel() : -1;
    if (page == m_page) return;
    m_page = page;
    if (page >= 0) {
        // A channel page starts on its own channel (or nothing, by the preference)
        const int inputs = m_ctx.device->numInputs();
        Selection s;
        s.tap = page < inputs ? RTA_TAP_INPUT : RTA_TAP_OUTPUT;
        if (m_prefs->value(QLatin1String(kChannelPagesShowSpectrum)).toBool())
            s.channels = { page < inputs ? page : page - inputs };
        m_pageSelection = s;
        m_pageOtherKnown = false;
    }
    emit selectionChanged();
    push();
}

// ---- Selections ----

QList<int> RtaController::available(int tap) const {
    QList<int> out;
    const ChannelListModel *rows = m_ctx.device->channels();
    if (!rows || !ready()) return out;
    const int capsLimit = tap == RTA_TAP_INPUT ? m_caps.input_channels : m_caps.output_channels;
    const int limit = std::min(capsLimit > 0 ? capsLimit : 16, 16);
    const int inputs = m_ctx.device->numInputs();
    for (int r = 0; r < rows->rowCount(); ++r) {
        const bool input = rows->get(r, QStringLiteral("kind")).toString() == QLatin1String("input");
        if (input != (tap == RTA_TAP_INPUT)) continue;
        const int wire = rows->get(r, QStringLiteral("channel")).toInt();
        const int ch = input ? wire : wire - inputs;
        if (ch < 0 || ch >= limit) continue;
        if (!input && !rows->get(r, QStringLiteral("outputEnabled")).toBool()) continue;
        out.append(ch);
    }
    return out;
}

RtaController::Selection RtaController::restricted(const Selection &s) const {
    const QList<int> avail = available(s.tap);
    Selection r{ s.tap, {} };
    for (int c : s.channels)
        if (avail.contains(c)) r.channels.append(c);
    return r;
}

RtaController::Selection RtaController::dashboardSelection() const {
    const QString key = m_prefs->value(QLatin1String(kDashboardSource)).toString();
    Selection stored;
    if (!key.isEmpty() && Selection::parse(key, &stored)) {
        // Nothing chosen on purpose stays nothing
        if (stored.channels.isEmpty()) return stored;
        const Selection r = restricted(stored);
        if (!r.channels.isEmpty()) return r;
    }
    // The first enabled output, else input 1
    const QList<int> outs = available(RTA_TAP_OUTPUT);
    if (!outs.isEmpty()) return { RTA_TAP_OUTPUT, { outs.first() } };
    return { RTA_TAP_INPUT, { 0 } };
}

RtaController::Selection RtaController::own() const {
    return m_page < 0 ? dashboardSelection() : restricted(m_pageSelection);
}

RtaController::Selection RtaController::drawn() const {
    if (!m_owner.following) return own();
    Selection s{ m_owner.tap == RTA_TAP_INPUT ? RTA_TAP_INPUT : RTA_TAP_OUTPUT, {} };
    for (int c = 0; c < 16; ++c)
        if (m_owner.mask & (1u << c)) s.channels.append(c);
    return s;
}

void RtaController::setPref(const char *key, const QVariant &value) {
    const QString k = QLatin1String(key);
    if (m_prefs->value(k) == value) return;
    QSettings().setValue(QStringLiteral("spectrum/") + k, value);
    m_prefs->insert(k, value);
}

void RtaController::storeDashboard(const Selection &s) { setPref(kDashboardSource, s.key()); }

void RtaController::setOwn(const Selection &s, bool takeBack) {
    if (m_page < 0) {
        storeDashboard(s);
    } else {
        m_pageSelection = s;
        setPref(kChannelPagesShowSpectrum, !s.channels.isEmpty());
    }
    emit selectionChanged();
    push();
    if (takeBack && m_owner.following) takeControl();
}

void RtaController::toggleChannel(int channel) {
    const Selection shown = drawn();
    Selection next = shown;
    if (next.channels.contains(channel)) next.channels.removeAll(channel);
    else next.channels.append(channel);
    std::sort(next.channels.begin(), next.channels.end());
    setOwn(next, true);
}

void RtaController::switchSide(int tap) {
    tap = tap == RTA_TAP_INPUT ? RTA_TAP_INPUT : RTA_TAP_OUTPUT;
    const Selection current = drawn();
    if (current.tap == tap && !m_owner.following) return;
    Selection entering{ tap, {} };
    if (m_page < 0) {
        Selection other;
        const QString key = m_prefs->value(QLatin1String(kDashboardOtherSide)).toString();
        if (Selection::parse(key, &other) && other.tap == tap) entering = other;
        Selection leaving;
        const QString stored = m_prefs->value(QLatin1String(kDashboardSource)).toString();
        if (!Selection::parse(stored, &leaving) || leaving.tap != current.tap) leaving = current;
        if (leaving.tap != tap) setPref(kDashboardOtherSide, leaving.key());
        storeDashboard(entering);
    } else {
        if (m_pageOtherKnown && m_pageOtherSide.tap == tap) entering = m_pageOtherSide;
        if (current.tap != tap) {
            m_pageOtherSide = m_pageSelection;
            m_pageOtherKnown = true;
        }
        m_pageSelection = entering;
    }
    emit selectionChanged();
    push();
    if (m_owner.following) takeControl();
}

void RtaController::clearSelection() { setOwn({ drawn().tap, {} }, true); }

void RtaController::takeControl() {
    m_ctx.core->worker()->post([](FfiCore *c) { dspi_rta_take_control(c); });
}

bool RtaController::showGraph() const {
    return m_prefs->value(QLatin1String(m_page < 0 ? kDashboardShowGraph : kChannelShowGraph)).toBool();
}
void RtaController::setShowGraph(bool on) {
    if (on == showGraph()) return;
    setPref(m_page < 0 ? kDashboardShowGraph : kChannelShowGraph, on);
    emit selectionChanged();
    push();
}
bool RtaController::showBars() const {
    return m_prefs->value(QLatin1String(m_page < 0 ? kDashboardShowBars : kChannelShowBars)).toBool();
}
void RtaController::setShowBars(bool on) {
    if (on == showBars()) return;
    setPref(m_page < 0 ? kDashboardShowBars : kChannelShowBars, on);
    emit selectionChanged();
    push();
}

QVariantMap RtaController::channelInfo(int tap, int channel) const {
    const int inputs = m_ctx.device->numInputs();
    const int wire = tap == RTA_TAP_INPUT ? channel : inputs + channel;
    QVariantMap m{ { QStringLiteral("channel"), channel }, { QStringLiteral("wire"), wire },
                   { QStringLiteral("kind"), tap == RTA_TAP_INPUT ? QStringLiteral("input") : QStringLiteral("output") },
                   { QStringLiteral("number"), channel }, { QStringLiteral("pdm"), false },
                   { QStringLiteral("name"), tr(QT_TRANSLATE_NOOP("RtaController", "Ch %1")).arg(channel + 1) } };
    const ChannelListModel *rows = m_ctx.device->channels();
    const int row = rows ? rows->rowOfChannel(wire) : -1;
    if (row >= 0) {
        const QString label = rows->get(row, QStringLiteral("label")).toString();
        if (!label.isEmpty()) m.insert(QStringLiteral("name"), label);
        m.insert(QStringLiteral("number"), rows->get(row, QStringLiteral("number")));
        m.insert(QStringLiteral("pdm"), rows->get(row, QStringLiteral("pdm")));
    }
    return m;
}

QVariantList RtaController::channels(int tap) const {
    QVariantList out;
    for (int c : available(tap)) out.append(channelInfo(tap, c));
    return out;
}

QString RtaController::pageTitle() const {
    if (m_page < 0) return tr(QT_TRANSLATE_NOOP("RtaController", "Dashboard"));
    const ChannelListModel *rows = m_ctx.device->channels();
    const int row = rows ? rows->rowOfChannel(m_page) : -1;
    const QString label = row >= 0 ? rows->get(row, QStringLiteral("label")).toString() : QString();
    return label.isEmpty() ? tr(QT_TRANSLATE_NOOP("RtaController", "Channel")) : label;
}

// ---- The request ----

void RtaController::setView(const QString &id, int kind, bool active) {
    const bool had = m_views.contains(id);
    if (active) m_views.insert(id, kind);
    else m_views.remove(id);
    if (had != active || (active && m_views.value(id) != kind)) push();
}

void RtaController::push() {
    if (!ready()) return;
    const Selection s = own();
    bool any = false, curves = false;
    for (auto it = m_views.cbegin(); it != m_views.cend(); ++it) {
        any = true;
        curves = curves || it.value() == Curves;
    }
    RtaRequest r{};
    r.tap = uint8_t(s.tap);
    r.mask = any ? s.mask() : 0;
    r.wants_bins = curves;
    r.fft_order = uint8_t(std::clamp(m_settings ? m_settings->value(QStringLiteral("fftOrder")).toInt() : 10, 8, 10));
    r.avg_ms = uint16_t(std::clamp(m_settings ? m_settings->value(QStringLiteral("avgMs")).toInt() : 300, 0, 65535));
    r.peak_decay_db_s =
        uint8_t(std::clamp(m_settings ? qRound(m_settings->value(QStringLiteral("peakDecay")).toDouble()) : 12, 0, 255));
    const bool same = m_sentOnce && r.tap == m_request.tap && r.mask == m_request.mask && r.wants_bins == m_request.wants_bins
        && r.fft_order == m_request.fft_order && r.avg_ms == m_request.avg_ms
        && r.peak_decay_db_s == m_request.peak_decay_db_s && any == m_watching;
    if (same) return;
    m_request = r;
    m_watching = any;
    m_sentOnce = true;
    ++m_requestsSent;
    m_ctx.core->worker()->post([r, any](FfiCore *c) { dspi_rta_set_request_watching(c, &r, any); });
    // The Link client starts the analyser again after a reconnect while it is wanted
    if (auto *link = Registrar::instance<LinkController>(); link && !link->boundSerial().isEmpty())
        link->setAnalyserWanted(true);
    emit requestChanged();
}

// ---- Scale and bands ----

bool RtaController::ready() const { return m_ctx.device->connected(); }

double RtaController::floorDb() const { return m_settings ? m_settings->value(QStringLiteral("floorDb")).toDouble() : -90.0; }
double RtaController::ceilingDb() const { return m_settings ? m_settings->value(QStringLiteral("ceilingDb")).toDouble() : 6.0; }
bool RtaController::showPeakHold() const { return m_settings ? m_settings->value(QStringLiteral("showPeakHold")).toBool() : true; }
double RtaController::graphOpacity() const {
    return std::clamp(m_settings ? m_settings->value(QStringLiteral("graphOpacity")).toDouble() : 1.0, 0.3, 1.0);
}
double RtaController::smoothing() const {
    const bool on = m_settings ? m_settings->value(QStringLiteral("smoothing")).toBool() : true;
    return on ? kSmoothingAmount : 0.0;
}
double RtaController::smoothingTau() const {
    const double amount = smoothing();
    if (amount <= 0) return 0;
    const double interval = std::isfinite(m_snap->refresh_interval_s) ? double(m_snap->refresh_interval_s) : 0.0;
    return std::clamp(interval * amount, 0.035, 0.40);
}

int RtaController::bandCount() const {
    int most = 0;
    for (int i = 0; i < m_snap->count && i < RTA_MAX_CHANNELS; ++i) most = std::max<int>(most, m_snap->frames[i].n_bands);
    if (most > 0) return std::min(most, RTA_MAX_BANDS);
    return std::min(std::max<int>(m_caps.max_bands, 34), RTA_MAX_BANDS);
}

bool RtaController::bandVisible(int band) const {
    if (band < 0 || band >= RTA_MAX_BANDS) return false;
    if (m_snap->sample_rate != 0) return (m_snap->populated >> band) & 1u;
    // No status read yet: the Mac's rule at 48 kHz and the requested size,
    // so the bars show at once
    const int bass = m_caps.bass_bands > 0 ? m_caps.bass_bands : 14;
    if (band < bass) return true;
    const double fc = 1000.0 * std::pow(10.0, (band - 20) / 10.0);
    const double lo = fc * std::pow(10.0, -0.05), hi = fc * std::pow(10.0, 0.05);
    const int order = m_request.fft_order ? m_request.fft_order : 10;
    const double n = double(1 << order);
    const double binHz = 48000.0 / n;
    return std::max(1.0, std::ceil(lo / binHz)) <= std::min(n / 2 - 1, std::floor(hi / binHz));
}

// ---- Texts ----

QString RtaController::shortHz(double hz) {
    if (hz >= 1000) return QStringLiteral("%1k").arg(qRound(hz / 1000.0));
    return QString::number(qRound(hz));
}

QStringList RtaController::statusParts() const {
    const RtaSnapshot &s = *m_snap;
    QStringList parts;
    if (!s.running || s.live_count == 0) {
        parts.append(tr(QT_TRANSLATE_NOOP("RtaController", "idle")));
    } else {
        const int ms = qRound(double(s.refresh_interval_s) * 1000.0);
        parts.append(s.live_count == 1
                         ? tr(QT_TRANSLATE_NOOP("RtaController", "1 channel, each refreshed every %1 ms")).arg(ms)
                         : tr(QT_TRANSLATE_NOOP("RtaController", "%1 channels, each refreshed every %2 ms")).arg(s.live_count).arg(ms));
    }
    if (s.frames_per_s > 0) parts.append(tr(QT_TRANSLATE_NOOP("RtaController", "%1 frames/s")).arg(s.frames_per_s));
    if (s.last_frame_us > 0) parts.append(tr(QT_TRANSLATE_NOOP("RtaController", "transform %1 µs")).arg(s.last_frame_us));
    if (s.busy_us_per_s > 0)
        parts.append(tr(QT_TRANSLATE_NOOP("RtaController", "main loop %1%")).arg(s.busy_us_per_s / 10000.0, 0, 'f', 1));
    return parts;
}

QString RtaController::bassText() const {
    const uint16_t b = m_snap->bass_busy_us_per_s;
    if (b == 0) return QString();
    const QString n = QString::number(b / 10000.0, 'f', 2);
    return tr(QT_TRANSLATE_NOOP("RtaController", "bass %1%")).arg(b == 0xFFFF ? QStringLiteral("≥") + n : n);
}

QString RtaController::lowBandText() const {
    const RtaSnapshot &s = *m_snap;
    const int order = m_settings ? m_settings->value(QStringLiteral("fftOrder")).toInt() : 10;
    if (!s.running || order >= m_caps.order_max || s.first_band == 0xFF || s.first_band < 1 || s.first_band >= m_caps.max_bands)
        return QString();
    return tr(QT_TRANSLATE_NOOP("RtaController", "shaded bands below %1 Hz need a larger transform size in Settings"))
        .arg(shortHz(m_caps.centres[s.first_band]));
}

QString RtaController::rangeText() const {
    if (m_caps.dynamic_range_db == 0) return QString();
    return tr(QT_TRANSLATE_NOOP("RtaController", "%1/%2 dB range")).arg(m_caps.dynamic_range_db).arg(m_caps.bass_dynamic_range_db);
}
