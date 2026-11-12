// -- WP QP10 -- The Settings window: pages, search, history, preferences
#include "SettingsController.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "PinMap.h"
#include "PrefsMap.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QCoreApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSettings>
#include <algorithm>
#include <array>

namespace SettingsRegistry {
std::vector<PageEntry> &pages() {
    static std::vector<PageEntry> list;
    return list;
}
std::vector<SectionEntry> &sections() {
    static std::vector<SectionEntry> list;
    return list;
}
std::vector<FooterEntry> &footers() {
    static std::vector<FooterEntry> list;
    return list;
}
} // namespace SettingsRegistry

namespace {

using SettingsRegistry::Page;

// The pages this package owns (PLAN QP10), with the Mac's titles, icons and
// order; the search terms are their rows' labels
const Page aboutPage{ "about", SettingsRegistry::Application, 10, QT_TRANSLATE_NOOP("SettingsPages", "About"), "info",
                      "AboutPage",
                      { QT_TRANSLATE_NOOP("SettingsPages", "Version"), QT_TRANSLATE_NOOP("SettingsPages", "Credits"),
                        QT_TRANSLATE_NOOP("SettingsPages", "Links & Support"),
                        QT_TRANSLATE_NOOP("SettingsPages", "Licences"), QT_TRANSLATE_NOOP("SettingsPages", "Notices"),
                        QT_TRANSLATE_NOOP("SettingsPages", "GitHub"), QT_TRANSLATE_NOOP("SettingsPages", "Weeb Labs") } };
const Page updatesPage{ "updates", SettingsRegistry::Application, 20, QT_TRANSLATE_NOOP("SettingsPages", "Updates"),
                        "download", "UpdatesPage",
                        { QT_TRANSLATE_NOOP("SettingsPages", "Check for Updates Automatically"),
                          QT_TRANSLATE_NOOP("SettingsPages", "Check Now"), QT_TRANSLATE_NOOP("SettingsPages", "Version"),
                          QT_TRANSLATE_NOOP("SettingsPages", "Release"), QT_TRANSLATE_NOOP("SettingsPages", "Privacy") } };
const Page advancedPage{ "advanced", SettingsRegistry::Application, 30, QT_TRANSLATE_NOOP("SettingsPages", "Advanced"),
                         "settings-2", "AdvancedPage",
                         { QT_TRANSLATE_NOOP("SettingsPages", "Channel Names"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Reset all channel names to factory defaults."),
                           QT_TRANSLATE_NOOP("SettingsPages", "Show Debug Information"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Diagnostics"), QT_TRANSLATE_NOOP("SettingsPages", "Firmware"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Serial Number"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Core Voltage") } };
const Page graphingPage{ "graphing", SettingsRegistry::Display, 10, QT_TRANSLATE_NOOP("SettingsPages", "Graphing"),
                         "chart-line", "GraphingPage",
                         { QT_TRANSLATE_NOOP("SettingsPages", "Graph Line Glow"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Show Phase Response"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Unwrap Phase"), QT_TRANSLATE_NOOP("SettingsPages", "Line Width"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Show Frequency Grid"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Show dB Grid"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Grid Opacity"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Vertical Range"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Min Frequency"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Max Frequency"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Readout"), QT_TRANSLATE_NOOP("SettingsPages", "Pop-out Window") } };
const Page spectrumPage{ "spectrum", SettingsRegistry::Display, 20,
                         QT_TRANSLATE_NOOP("SettingsPages", "Spectrum Analyser"), "audio-waveform", "SpectrumPage",
                         { QT_TRANSLATE_NOOP("SettingsPages", "Spectrum Strength"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Peak Hold"), QT_TRANSLATE_NOOP("SettingsPages", "Smoothing"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Floor"), QT_TRANSLATE_NOOP("SettingsPages", "Ceiling"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Transform Size"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Averaging"), QT_TRANSLATE_NOOP("SettingsPages", "Peak Decay"),
                           QT_TRANSLATE_NOOP("SettingsPages", "RTA"), QT_TRANSLATE_NOOP("SettingsPages", "FFT") } };
const Page overviewPage{ "overview", SettingsRegistry::System, 10, QT_TRANSLATE_NOOP("SettingsPages", "Overview"), "cpu",
                         "OverviewPage",
                         { QT_TRANSLATE_NOOP("SettingsPages", "Pins"), QT_TRANSLATE_NOOP("SettingsPages", "GPIO"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Core Voltage"), QT_TRANSLATE_NOOP("SettingsPages", "Outputs"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Clocks"), QT_TRANSLATE_NOOP("SettingsPages", "Inputs"),
                           QT_TRANSLATE_NOOP("SettingsPages", "Control"), QT_TRANSLATE_NOOP("SettingsPages", "Hangs") } };

const Registrar::Controller<SettingsController> settingsController;
const Registrar::Window settingsWindow("edit.settings", "SettingsWindow");

const char *const kLastPageKey = "settingsWindow/lastPage";
const char *const kDebugKey = "advanced/showDebugInfo";
const char *const kFirstPage = "about";

QString translated(const char *text) { return QCoreApplication::translate("SettingsPages", text); }

// The page entries of one group, in order
std::vector<const SettingsRegistry::PageEntry *> groupPages(SettingsRegistry::Group group) {
    std::vector<const SettingsRegistry::PageEntry *> list;
    for (const auto &p : SettingsRegistry::pages())
        if (p.group == group) list.push_back(&p);
    std::stable_sort(list.begin(), list.end(), [](auto *a, auto *b) {
        return a->order != b->order ? a->order < b->order : qstrcmp(a->id, b->id) < 0;
    });
    return list;
}

} // namespace

SettingsController::SettingsController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_spectrum(new PrefsMap(QStringLiteral("spectrum"), this)) {
    // Spectrum Analyser
    const std::array<std::pair<const char *, QVariant>, 8> spectrum{ {
        { "fftOrder", 10 }, { "avgMs", 300 }, { "peakDecay", 12.0 }, { "floorDb", -90.0 },
        { "ceilingDb", 6.0 }, { "smoothing", true }, { "showPeakHold", true }, { "graphOpacity", 1.0 },
    } };
    for (const auto &[key, value] : spectrum) m_spectrum->declare(QString::fromLatin1(key), value);

    QSettings settings;
    m_showDebugInfo = settings.value(QLatin1String(kDebugKey), false).toBool();
    m_current = settings.value(QLatin1String(kLastPageKey), QLatin1String(kFirstPage)).toString();
    if (!entry(m_current)) m_current = QLatin1String(kFirstPage);
    m_history = { m_current };

    connect(ctx.device, &DeviceModel::snapshotApplied, this, &SettingsController::onSnapshot);
    // Help > About opens Settings on its About page
    ctx.shell->addHandler(QStringLiteral("help.about"), this, [this]() { showPage(QLatin1String(kFirstPage)); });
}

SettingsController::~SettingsController() = default;

const SettingsRegistry::PageEntry *SettingsController::entry(const QString &id) const {
    for (const auto &p : SettingsRegistry::pages())
        if (id == QLatin1String(p.id)) return &p;
    return nullptr;
}

QStringList SettingsController::footerTypes() const {
    auto list = SettingsRegistry::footers();
    std::stable_sort(list.begin(), list.end(), [](const auto &a, const auto &b) { return a.order < b.order; });
    QStringList types;
    for (const auto &f : list) types.append(QString::fromLatin1(f.type));
    return types;
}

QStringList SettingsController::sectionTypes(const QString &id) const {
    std::vector<SettingsRegistry::SectionEntry> list;
    for (const auto &s : SettingsRegistry::sections())
        if (id == QLatin1String(s.page)) list.push_back(s);
    std::stable_sort(list.begin(), list.end(), [](const auto &a, const auto &b) { return a.order < b.order; });
    QStringList types;
    for (const auto &s : list) types.append(QString::fromLatin1(s.type));
    return types;
}

QString SettingsController::pageTitle(const QString &id) const {
    const auto *p = entry(id);
    return p ? translated(p->title) : QString();
}

QString SettingsController::pageType(const QString &id) const {
    const auto *p = entry(id);
    return p ? QString::fromLatin1(p->type) : QString();
}

QString SettingsController::pageIcon(const QString &id) const {
    const auto *p = entry(id);
    return p ? QString::fromLatin1(p->icon) : QString();
}

bool SettingsController::isShown(const QString &id) const {
    const auto *p = entry(id);
    if (!p || m_unavailable.value(id)) return false;
    // System and Control describe the open device (behaviour.md §8)
    if (p->group == SettingsRegistry::System || p->group == SettingsRegistry::Control) return m_ctx.device->connected();
    return true;
}

bool SettingsController::matches(const QString &id, const QString &query) const {
    const QString q = query.trimmed();
    if (q.isEmpty()) return true;
    const auto *p = entry(id);
    if (!p) return false;
    if (translated(p->title).contains(q, Qt::CaseInsensitive)) return true;
    for (const char *term : p->terms)
        if (translated(term).contains(q, Qt::CaseInsensitive)) return true;
    for (const QString &term : m_extraTerms.value(id))
        if (term.contains(q, Qt::CaseInsensitive)) return true;
    return false;
}

QVariantList SettingsController::sidebarRows(const QString &query) const {
    static const std::array<const char *, 4> titles{ QT_TR_NOOP("Application"), QT_TR_NOOP("Display"),
                                                     QT_TR_NOOP("System"), QT_TR_NOOP("Control") };
    QVariantList rows;
    for (int g = SettingsRegistry::Application; g <= SettingsRegistry::Control; ++g) {
        QVariantList pages;
        for (const auto *p : groupPages(SettingsRegistry::Group(g))) {
            const QString id = QString::fromLatin1(p->id);
            if (!isShown(id) || !matches(id, query)) continue;
            pages.append(QVariantMap{ { QStringLiteral("id"), id },
                                      { QStringLiteral("title"), translated(p->title) },
                                      { QStringLiteral("icon"), QString::fromLatin1(p->icon) } });
        }
        if (pages.isEmpty()) continue;
        rows.append(QVariantMap{ { QStringLiteral("heading"), tr(titles[size_t(g)]) } });
        rows.append(pages);
    }
    return rows;
}

void SettingsController::setPageAvailable(const QString &id, bool available) {
    if (m_unavailable.value(id) == !available) return;
    m_unavailable.insert(id, !available);
    ++m_pagesRevision;
    emit pagesChanged();
    if (!available && id == m_current) ensureShown();
}

void SettingsController::addSearchTerms(const QString &id, const QStringList &terms) {
    m_extraTerms[id].append(terms);
    ++m_pagesRevision;
    emit pagesChanged();
}

void SettingsController::setUpdater(QObject *updater) {
    if (updater == m_updater) return;
    m_updater = updater;
    emit updaterChanged();
}

void SettingsController::setShowDebugInfo(bool on) {
    if (on == m_showDebugInfo) return;
    m_showDebugInfo = on;
    QSettings().setValue(QLatin1String(kDebugKey), on);
    emit showDebugInfoChanged();
}

// ---- Navigation (the Mac's history: back and forward through visited pages) ----

void SettingsController::navigate(const QString &id, bool record) {
    if (record) {
        if (id == m_current) return;
        // A fresh navigation drops the forward entries
        while (m_history.size() > m_historyIndex + 1) m_history.removeLast();
        m_history.append(id);
        m_historyIndex = int(m_history.size()) - 1;
    }
    m_current = id;
    QSettings().setValue(QLatin1String(kLastPageKey), id);
    emit currentPageChanged();
    if (id == QLatin1String("overview") || id == QLatin1String("spectrum") || id == QLatin1String("advanced"))
        refreshDeviceFacts();
}

void SettingsController::open(const QString &id) {
    if (!entry(id)) return;
    navigate(id, true);
}

void SettingsController::back() {
    if (!canGoBack()) return;
    --m_historyIndex;
    navigate(m_history.at(m_historyIndex), false);
}

void SettingsController::forward() {
    if (!canGoForward()) return;
    ++m_historyIndex;
    navigate(m_history.at(m_historyIndex), false);
}

void SettingsController::ensureShown() {
    if (!isShown(m_current)) navigate(QLatin1String(kFirstPage), true);
}

void SettingsController::showPage(const QString &id) {
    open(id);
    m_ctx.shell->openWindow(QStringLiteral("edit.settings"));
}

// ---- Device facts --------------------------------------------------------------

void SettingsController::onSnapshot() {
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    if (snap.connected != m_connected) {
        m_connected = snap.connected;
        ++m_pagesRevision;
        emit pagesChanged();
        ensureShown();
    }
    if (snap.generation != m_factsGeneration) {
        // Another device (or none): what was read of the last one is gone
        m_factsGeneration = snap.generation;
        m_coreVoltage.clear();
        m_analyser.clear();
        emit coreVoltageChanged();
        emit analyserChanged();
        if (m_connected && (m_current == QLatin1String("overview") || m_current == QLatin1String("spectrum") ||
                            m_current == QLatin1String("advanced")))
            refreshDeviceFacts();
    }
    readPins();
}

void SettingsController::readPins() {
    if (!m_ctx.device->state()) {
        if (!m_pinClaims.isEmpty()) {
            m_pinClaims.clear();
            emit pinsChanged();
        }
        return;
    }
    // The surfaces' live bindings are the core's cache, read under its lock
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            QList<PinMap::SurfacePin> uses;
            std::array<uint8_t, 64> pins{}, owners{};
            const uint32_t n = std::min<uint32_t>(dspi_cs_pin_uses(c, pins.data(), owners.data(), pins.size()),
                                                  uint32_t(pins.size()));
            for (uint32_t i = 0; i < n; ++i) uses.append({ pins[i], owners[i] });
            return uses;
        },
        [this](const QList<PinMap::SurfacePin> &uses) {
            const DspState *state = m_ctx.device->state();
            QVariantList list;
            if (state) {
                for (const PinMap::Claim &c : PinMap::claims(*state, uses))
                    list.append(QVariantMap{ { QStringLiteral("pin"), c.pin },
                                             { QStringLiteral("role"), c.role },
                                             { QStringLiteral("label"), c.label } });
            }
            if (list == m_pinClaims) return;
            m_pinClaims = list;
            emit pinsChanged();
        });
}

QVariantList SettingsController::gpioPins() const {
    QVariantList list;
    for (int p : PinMap::gpioPins()) list.append(p);
    return list;
}

void SettingsController::refreshDeviceFacts() {
    if (!m_ctx.device->connected()) return;
    refreshCoreVoltage();
    readPins();
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            RtaCapsInfo caps{};
            dspi_rta_get_caps(c, &caps);
            return caps;
        },
        [this](const RtaCapsInfo &caps) {
            m_analyser = { { QStringLiteral("supported"), caps.supported },
                           { QStringLiteral("orderMin"), caps.order_min },
                           { QStringLiteral("orderMax"), caps.order_max },
                           { QStringLiteral("dynamicRangeDb"), caps.dynamic_range_db } };
            emit analyserChanged();
        });
}

void SettingsController::refreshCoreVoltage() {
    if (!m_ctx.device->connected()) return;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            QJsonObject status =
                QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_ext_status(c, b, n); }))
                    .object();
            QJsonObject vreg;
            if (status.value(QLatin1String("hasCoreVreg")).toBool())
                vreg = QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_fetch_core_vreg(c, b, n); }))
                           .object();
            return std::make_pair(status.value(QLatin1String("hasCoreVreg")).toBool(), vreg);
        },
        [this](const std::pair<bool, QJsonObject> &reply) {
            QVariantMap map = reply.second.toVariantMap();
            const bool read = reply.second.value(QLatin1String("ok")).toBool();
            map.insert(QStringLiteral("supported"), reply.first);
            map.insert(QStringLiteral("read"), read);
            if (!read && reply.first) map.insert(QStringLiteral("error"), reply.second.value(QLatin1String("error")).toString());
            m_coreVoltage = map;
            emit coreVoltageChanged();
        });
}

QString SettingsController::voltsLabel(int code) const {
    const bool rp2040 = m_ctx.device->platformName() == QLatin1String("RP2040");
    // RP2350 codes above 1.30 V (the Mac's VregVoltage table)
    static const std::array<double, 16> high{ 1.35, 1.40, 1.50, 1.60, 1.65, 1.70, 1.80, 1.90,
                                              2.00, 2.35, 2.50, 2.65, 2.80, 3.00, 3.15, 3.30 };
    double volts = -1;
    if (code >= 0x00 && code <= 0x0F) volts = rp2040 && code < 0x06 ? 0.80 : 0.55 + 0.05 * code;
    else if (code >= 0x10 && code <= 0x1F && !rp2040) volts = high[size_t(code - 0x10)];
    if (volts < 0) return tr("code 0x%1").arg(QString::number(code, 16).toUpper().rightJustified(2, QLatin1Char('0')));
    return tr("%1 V").arg(volts, 0, 'f', 2);
}

QString SettingsController::uptimeLabel(qint64 seconds) const {
    const qint64 s = std::max<qint64>(seconds, 0);
    const qint64 d = s / 86400, h = (s % 86400) / 3600, m = (s % 3600) / 60, sec = s % 60;
    if (d > 0) return tr("%1 d %2 h").arg(d).arg(h);
    if (h > 0) return tr("%1 h %2 min").arg(h).arg(m);
    if (m > 0) return tr("%1 min %2 s").arg(m).arg(sec);
    return tr("%1 s").arg(sec);
}

// The name the firmware gives wire channel `ch` (get_default_channel_name):
// inputs after the source's model, outputs "<SPDIF|I2S> <slot> <L|R>", the
// PDM sub last. Device data, not translated.
static QString defaultChannelName(const DspState &s, int ch) {
    const auto side = [](int n) { return n % 2 == 0 ? QLatin1Char('L') : QLatin1Char('R'); };
    if (ch < s.num_input_channels) {
        switch (s.input_source) {
        case 1: return QStringLiteral("SPDIF %1").arg(side(ch));
        case 2: return QStringLiteral("I2S %1 %2").arg(ch / 2 + 1).arg(side(ch));
        case 3: return QStringLiteral("ADAT %1").arg(ch + 1);
        case 4: case 5: case 6: return QStringLiteral("SPDIF %1 %2").arg(s.input_source - 2).arg(side(ch));
        default: return QStringLiteral("USB %1").arg(ch + 1);
        }
    }
    if (ch == s.num_channels - 1) return QStringLiteral("PDM");
    const int slot = (ch - s.num_input_channels) / 2;
    const bool i2s = slot < 4 && s.output_types[slot] == 1;
    return QStringLiteral("%1 %2 %3").arg(i2s ? QLatin1String("I2S") : QLatin1String("SPDIF")).arg(slot + 1).arg(side(ch - s.num_input_channels));
}

void SettingsController::resetChannelNames() {
    const DspState *state = m_ctx.device->state();
    if (!state) return;
    const DspState s = *state;
    for (int ch = 0; ch < std::min<int>(s.num_channels, MAX_CHANNELS); ++ch)
        m_ctx.device->setChannelName(ch, defaultChannelName(s, ch));
}
