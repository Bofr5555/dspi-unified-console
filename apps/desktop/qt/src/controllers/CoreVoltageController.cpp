// -- WP QP23 -- The Core Voltage page's controller (CoreVoltageController.h).
#include "CoreVoltageController.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "ExtController.h"
#include "SettingsController.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <algorithm>
#include <utility>

namespace {

const Registrar::Controller<CoreVoltageController> coreVoltageController;

// Not in the sidebar: Settings > Advanced > Core Voltage "Open…" is the only way
// in (behaviour.md §20). The mode manifest can drop the page by this id.
const SettingsRegistry::Page coreVoltagePage{
    "coreVoltage", SettingsRegistry::Hidden, 10, QT_TRANSLATE_NOOP("SettingsPages", "Core Voltage"), "gauge",
    "CoreVoltagePage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Calibrate"), QT_TRANSLATE_NOOP("SettingsPages", "Soak"),
      QT_TRANSLATE_NOOP("SettingsPages", "Trial"), QT_TRANSLATE_NOOP("SettingsPages", "Hangs") }
};
const char *const kPageId = "coreVoltage";

// The view is the warning, the soak note and the choices: past the default 1 KiB,
// and a poll must not be asked twice for it
constexpr uint32_t kViewBytes = 16384;

using Reply = std::pair<QByteArray, QByteArray>;

// On the worker: the flow (moved on by a read when `poll`), and while the flow
// cannot use the device (read-only, a hub's viewer) the plain status to show
Reply readFlow(FfiCore *c, bool poll) {
    const QByteArray view = readSized(
        [c, poll](char *b, uint32_t n) { return poll ? dspi_core_vreg_cal_poll(c, b, n) : dspi_core_vreg_cal_view(c, b, n); },
        kViewBytes);
    QByteArray status;
    if (QJsonDocument::fromJson(view).object().value(QLatin1String("status")).isObject()) return { view, status };
    const QJsonObject ext =
        QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_ext_status(c, b, n); })).object();
    if (ext.value(QLatin1String("hasCoreVreg")).toBool())
        status = readSized([c](char *b, uint32_t n) { return dspi_fetch_core_vreg(c, b, n); });
    return { view, status };
}

} // namespace

CoreVoltageController::CoreVoltageController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_poll = ctx.poller->add(QStringLiteral("core voltage"), 1000, this,
                             [](FfiCore *c) { return readFlow(c, true); },
                             [this](const Reply &r) { apply(r.first, r.second); });
    ctx.poller->setEnabled(m_poll, false);
    connect(ctx.device, &DeviceModel::stateChanged, this, &CoreVoltageController::onState);
}

void CoreVoltageController::onState() {
    // The extension is the connect probe's answer (QP7's ExtController); looked
    // up on first use, as controllers are made in no set order
    if (!m_ext) {
        m_ext = Registrar::instance<ExtController>();
        if (m_ext) connect(m_ext, &ExtController::changed, this, &CoreVoltageController::onExt);
    }
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    if (!snap.connected) {
        if (!m_open) return;
        m_open = false;
        m_view.clear();
        m_status.clear();
        onExt();
        updatePage();
        emit flowChanged();
        return;
    }
    if (m_open && snap.generation == m_generation) return;
    // A new connection: the core asks for a poll on every connect, so a trial
    // left running on the device shows as found
    m_open = true;
    m_generation = snap.generation;
    m_view.clear();
    m_status.clear();
    onExt();
    refresh();
}

void CoreVoltageController::refresh() {
    if (!m_ctx.device->connected()) return;
    m_ctx.core->worker()->call(this, [](FfiCore *c) { return readFlow(c, true); },
                               [this](const Reply &r) { apply(r.first, r.second); });
}

void CoreVoltageController::apply(const QByteArray &view, const QByteArray &status) {
    if (!m_ctx.device->connected()) return;
    const QJsonObject v = QJsonDocument::fromJson(view).object();
    if (v.value(QLatin1String("ok")).toBool()) m_view = v.toVariantMap();
    const QJsonObject s = v.value(QLatin1String("status")).isObject()
                              ? v.value(QLatin1String("status")).toObject()
                              : QJsonDocument::fromJson(status).object();
    m_status = s.toVariantMap();
    m_status.remove(QStringLiteral("ok"));
    updatePage();
    emit flowChanged();
}

void CoreVoltageController::onExt() {
    const bool has = m_ext && m_ext->known() && m_ext->hasCoreVreg() && m_ctx.device->connected();
    if (has == m_supported) return;
    m_supported = has;
    updatePage();
    if (has) refresh();
    emit flowChanged();
}

void CoreVoltageController::updatePage() {
    m_ctx.poller->setEnabled(m_poll, m_pageShown && m_supported);
    if (auto *settings = Registrar::instance<SettingsController>())
        settings->setPageAvailable(QLatin1String(kPageId), m_supported);
}

void CoreVoltageController::setPageShown(bool shown) {
    if (shown == m_pageShown) return;
    m_pageShown = shown;
    updatePage();
    if (shown) refresh();
    emit pageShownChanged();
}

QString CoreVoltageController::phase() const {
    const QString p = m_view.value(QStringLiteral("phase")).toString();
    return p.isEmpty() ? QStringLiteral("idle") : p;
}

bool CoreVoltageController::calibrating() const {
    // The core's Phase::busy: a manual request would disturb these
    static const QStringList running{ QStringLiteral("clearing"), QStringLiteral("starting_trial"),
                                      QStringLiteral("soaking"),  QStringLiteral("passed"),
                                      QStringLiteral("ending"),   QStringLiteral("committing") };
    return running.contains(phase());
}

bool CoreVoltageController::writable() const {
    return m_supported && m_ctx.device->connected() && !m_ctx.device->readOnly() &&
           m_view.value(QStringLiteral("available")).toBool();
}

QVariantList CoreVoltageController::soakChoices() const {
    const int minimum = m_view.value(QStringLiteral("minSoakS")).toInt();
    const int suggested = m_view.value(QStringLiteral("defaultSoakS")).toInt();
    if (minimum <= 0 || suggested <= 0) return {};
    // The core's shortest and suggested soaks, and longer ones for a cautious run
    QList<int> list{ minimum, suggested, 2 * suggested, 4 * suggested };
    std::sort(list.begin(), list.end());
    list.erase(std::unique(list.begin(), list.end()), list.end());
    QVariantList out;
    for (int s : list)
        if (s >= minimum) out.append(s);
    return out;
}

QString CoreVoltageController::stateName(const QString &key) const {
    if (key == QLatin1String("idle")) return tr("Not started");
    if (key == QLatin1String("review")) return tr("Ready");
    if (key == QLatin1String("clearing")) return tr("Clearing the record");
    if (key == QLatin1String("starting_trial")) return tr("Starting the trial");
    if (key == QLatin1String("soaking")) return tr("Soaking");
    if (key == QLatin1String("passed")) return tr("Passed");
    if (key == QLatin1String("failed")) return tr("Failed");
    if (key == QLatin1String("unknown")) return tr("Result unknown");
    if (key == QLatin1String("trial_found")) return tr("Trial found");
    if (key == QLatin1String("trial_left_running")) return tr("Trial left running");
    if (key == QLatin1String("ending")) return tr("Ending the trial");
    if (key == QLatin1String("ended")) return tr("Trial ended");
    if (key == QLatin1String("committing")) return tr("Saving");
    if (key == QLatin1String("committed")) return tr("Saved");
    if (key == QLatin1String("verify_failed")) return tr("Not confirmed");
    return key;   // a phase newer than this app: the core's key
}

QString CoreVoltageController::durationLabel(int seconds) const {
    const int s = std::max(seconds, 0);
    const int h = s / 3600, m = (s % 3600) / 60, sec = s % 60;
    if (h > 0) return m > 0 ? tr("%1 h %2 min").arg(h).arg(m) : tr("%1 h").arg(h);
    if (m > 0) return sec > 0 ? tr("%1 min %2 s").arg(m).arg(sec) : tr("%1 min").arg(m);
    return tr("%1 s").arg(sec);
}

bool CoreVoltageController::needsWord(const QString &action, int code) const {
    if (action == QLatin1String("commit")) return m_view.value(QStringLiteral("commitNeedsWord")).toBool();
    if (action == QLatin1String("reset")) return m_view.value(QStringLiteral("resetNeedsWord")).toBool();
    const char *field = action == QLatin1String("set") ? "saveNeedsWord"
                        : action == QLatin1String("start") || action == QLatin1String("trial") ? "needsWord"
                                                                                                : nullptr;
    if (!field) return false;
    for (const QVariant &c : m_view.value(QStringLiteral("choices")).toList()) {
        const QVariantMap choice = c.toMap();
        if (choice.value(QStringLiteral("code")).toInt() == code) return choice.value(QLatin1String(field)).toBool();
    }
    // A voltage the core does not offer: it refuses it; ask for the word anyway
    return true;
}

bool CoreVoltageController::wordMatches(const QString &typed) const {
    const QString word = m_view.value(QStringLiteral("confirmationWord")).toString();
    return !word.isEmpty() && typed == word;
}

// ---- Requests ------------------------------------------------------------------

void CoreVoltageController::request(const QString &what, std::function<uint8_t(FfiCore *)> setter) {
    ++m_busy;
    ++m_requests;
    emit flowChanged();
    m_ctx.device->write(
        what, [setter = std::move(setter)](FfiCore *c) { return setter(c) == 0; },
        [this, what](bool ok) {
            --m_busy;
            // The flow as the request left it, with the core's reason for a refusal
            m_ctx.core->worker()->call(this, [](FfiCore *c) { return readFlow(c, false); },
                                       [this, what, ok](const Reply &r) {
                                           apply(r.first, r.second);
                                           if (!ok)
                                               emit refused(what, m_view.value(QStringLiteral("lastError")).toString());
                                       });
            emit flowChanged();
        });
}

namespace {
// What the person typed, for the core: null when nothing was
struct Typed {
    explicit Typed(const QString &s) : bytes(s.toUtf8()), given(!s.isEmpty()) {}
    const char *get() const { return given ? bytes.constData() : nullptr; }
    QByteArray bytes;
    bool given;
};
uint8_t code8(int code) { return uint8_t(std::clamp(code, 0, 255)); }
} // namespace

void CoreVoltageController::openFlow() {
    request(QStringLiteral("core voltage open"), [](FfiCore *c) { return dspi_core_vreg_cal_open(c); });
}

void CoreVoltageController::closeFlow() {
    request(QStringLiteral("core voltage close"), [](FfiCore *c) { return dspi_core_vreg_cal_close(c); });
}

void CoreVoltageController::start(int code, int soakS, const QString &typed) {
    const Typed word(typed);
    const uint32_t soak = uint32_t(std::max(soakS, 0));
    request(QStringLiteral("core voltage start"),
            [word, code, soak](FfiCore *c) { return dspi_core_vreg_cal_start(c, code8(code), soak, word.get()); });
}

void CoreVoltageController::keep(int soakS) {
    const uint32_t soak = uint32_t(std::max(soakS, 0));
    request(QStringLiteral("core voltage keep"), [soak](FfiCore *c) { return dspi_core_vreg_cal_keep(c, soak); });
}

void CoreVoltageController::commit(const QString &typed) {
    const Typed word(typed);
    request(QStringLiteral("core voltage commit"), [word](FfiCore *c) { return dspi_core_vreg_cal_commit(c, word.get()); });
}

void CoreVoltageController::endTrial() {
    request(QStringLiteral("core voltage end trial"), [](FfiCore *c) { return dspi_core_vreg_trial_end(c); });
}

void CoreVoltageController::manualSet(int code, const QString &typed) {
    const Typed word(typed);
    request(QStringLiteral("core voltage set"),
            [word, code](FfiCore *c) { return dspi_core_vreg_manual_set(c, code8(code), word.get()); });
}

void CoreVoltageController::manualTrial(int code, const QString &typed) {
    const Typed word(typed);
    request(QStringLiteral("core voltage trial"),
            [word, code](FfiCore *c) { return dspi_core_vreg_manual_trial(c, code8(code), word.get()); });
}

void CoreVoltageController::manualAuto(bool on) {
    request(QStringLiteral("core voltage auto"), [on](FfiCore *c) { return dspi_core_vreg_manual_auto(c, on); });
}

void CoreVoltageController::manualClear() {
    request(QStringLiteral("core voltage clear"), [](FfiCore *c) { return dspi_core_vreg_manual_clear(c); });
}

void CoreVoltageController::demoEvent(int event, uint32_t seconds) {
    if (!m_ctx.device->connected()) return;
    m_ctx.core->worker()->call(
        this,
        [event, seconds](FfiCore *c) {
            dspi_core_vreg_demo_event(c, code8(event), seconds);
            return readFlow(c, true);
        },
        [this](const Reply &r) { apply(r.first, r.second); });
}
