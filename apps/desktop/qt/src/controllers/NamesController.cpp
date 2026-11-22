// -- WP QP11 -- The device's and the input sources' names (NamesController.h).
#include "NamesController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "SessionState.h"

#include <QPointer>
#include <QVector>

static const Registrar::Controller<NamesController> namesController;

namespace {
enum Source { Usb = 0, Spdif = 1, I2s = 2, Adat = 3, Spdif2 = 4 };
constexpr int kSourceLast = 6;    // INPUT_SOURCE_SPDIF4, the last source the firmware has
constexpr int kNoPin = 0xFF;

struct Names {
    bool sources = false;
    bool device = false;
    QHash<int, QString> stored;
    QString deviceName;
};
} // namespace

NamesController::NamesController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(m_ctx.device, &DeviceModel::snapshotApplied, this, &NamesController::onSnapshot);
}

// -- WP MDQT2a --
std::shared_ptr<NamesController::State> NamesController::state() const {
    return sessionState<State>(m_ctx.activeSession(), QStringLiteral("NamesController"));
}

void NamesController::onSnapshot() {
    const std::shared_ptr<State> st = state();
    const bool moved = st != m_shown;   // -- WP MDQT2a -- another device is the active one
    m_shown = st;
    if (!m_ctx.device->connected()) {
        st->sourcesSupported = st->deviceSupported = false;
        if (moved) touched();
        else emit changed();
        return;
    }
    // Drafts belong to the device they were typed for
    if (m_ctx.device->serial() != st->serial) {
        st->serial = m_ctx.device->serial();
        st->drafts.clear();
        st->deviceDraftSet = false;
    }
    if (moved) touched();
    readNames(m_ctx.activeSession(), st);
}

// The core's cached names (no traffic); a rename from another console arrives
// as a notification and a snapshot, which reads them again
void NamesController::readNames(DeviceSession *session, const std::shared_ptr<State> &st) {
    session->worker()->call(
        this,
        [](FfiCore *c) {
            Names n;
            char buf[64] = {};
            n.sources = dspi_cached_input_source_name(c, 0, buf, sizeof buf);
            if (n.sources) {
                for (int s = 0; s <= kSourceLast; ++s) {
                    buf[0] = 0;
                    if (dspi_cached_input_source_name(c, uint8_t(s), buf, sizeof buf)) n.stored.insert(s, QString::fromUtf8(buf));
                }
            }
            buf[0] = 0;
            n.device = dspi_cached_device_name(c, buf, sizeof buf);
            n.deviceName = QString::fromUtf8(buf);
            return n;
        },
        [this, st](const Names &n) {
            st->sourcesSupported = n.sources;
            st->deviceSupported = n.device;
            st->stored = n.stored;
            st->storedDevice = n.deviceName;
            // A draft that now equals the stored name is no longer an edit
            for (auto it = st->drafts.begin(); it != st->drafts.end();) {
                if (it.value() == st->stored.value(it.key())) it = st->drafts.erase(it);
                else ++it;
            }
            if (st->deviceDraftSet && st->deviceDraft == st->storedDevice) st->deviceDraftSet = false;
            if (st == state()) touched();   // -- WP MDQT2a --
        });
}

QString NamesController::deviceName() const {
    const std::shared_ptr<State> st = state();
    return st->deviceDraftSet ? st->deviceDraft : st->storedDevice;
}

QList<int> NamesController::sourceOptions() const {
    QList<int> out{ Usb, Spdif };
    auto *hw = Registrar::instance<HardwareController>();
    if (!hw) return out;
    const QVariantMap has = hw->has();
    const QVariantMap state = hw->hw();
    if (has.value(QStringLiteral("multiSpdif")).toBool()) {
        const QVariantList enabled = state.value(QStringLiteral("spdifEnabled")).toList();
        for (int i = 1; i < state.value(QStringLiteral("spdifCount")).toInt(); ++i)
            if (enabled.value(i).toBool()) out << Spdif2 + i - 1;
    }
    if (has.value(QStringLiteral("i2sInput")).toBool()) out << I2s;
    // ADAT only once it is on with a pin: the firmware ignores it otherwise
    if (has.value(QStringLiteral("adatIn")).toBool() && state.value(QStringLiteral("adatInEnabled")).toBool()
        && state.value(QStringLiteral("adatInPin")).toInt() != kNoPin)
        out << Adat;
    return out;
}

QString NamesController::builtInTitle(int source) const {
    auto *hw = Registrar::instance<HardwareController>();
    const QVariantList enabled = hw ? hw->hw().value(QStringLiteral("spdifEnabled")).toList() : QVariantList();
    bool optional = false;
    for (int i = 1; i < enabled.size(); ++i) optional = optional || enabled.value(i).toBool();
    switch (source) {
    case Usb: return tr("USB");
    case Spdif: return optional ? tr("S/PDIF 1") : tr("S/PDIF");
    case Spdif2:
    case Spdif2 + 1:
    case Spdif2 + 2: return tr("S/PDIF %1").arg(source - Spdif2 + 2);
    case I2s: return tr("I2S");
    case Adat: return tr("ADAT");
    default: return tr("Source %1").arg(source);
    }
}

QVariantList NamesController::sources() const {
    QVariantList out;
    auto *hw = Registrar::instance<HardwareController>();
    const QVariantMap state = hw ? hw->hw() : QVariantMap();
    const QVariantList spdifPins = state.value(QStringLiteral("spdifPins")).toList();
    const QHash<int, QString> pending = pendingSources();
    const std::shared_ptr<State> st = this->state();
    for (int source : sourceOptions()) {
        // Where the source comes in, so similar ones can be told apart
        QString detail;
        switch (source) {
        case Usb: detail = tr("Audio over USB"); break;
        case I2s: detail = tr("External I2S device"); break;
        case Adat: detail = tr("Lightpipe on GPIO %1").arg(state.value(QStringLiteral("adatInPin")).toInt()); break;
        default: {
            const int index = source == Spdif ? 0 : source - Spdif2 + 1;
            detail = index < spdifPins.size() ? tr("Receiver on GPIO %1").arg(spdifPins.value(index).toInt())
                                              : tr("S/PDIF receiver");
            break;
        }
        }
        const QString stored = st->stored.value(source);
        out.append(QVariantMap{ { QStringLiteral("source"), source },
                                { QStringLiteral("title"), builtInTitle(source) },
                                { QStringLiteral("detail"), detail },
                                { QStringLiteral("stored"), stored },
                                { QStringLiteral("text"), st->drafts.value(source, stored) },
                                { QStringLiteral("edited"), pending.contains(source) } });
    }
    return out;
}

QString NamesController::setSourceName(int source, const QString &text) {
    const QString clean = HardwareController::sanitizeName(text);
    const std::shared_ptr<State> st = state();
    if (clean == st->stored.value(source)) st->drafts.remove(source);
    else st->drafts.insert(source, clean);
    touched();
    return clean;
}

QString NamesController::setDeviceName(const QString &text) {
    const QString clean = HardwareController::sanitizeName(text);
    const std::shared_ptr<State> st = state();
    st->deviceDraftSet = clean != st->storedDevice;
    st->deviceDraft = clean;
    touched();
    return clean;
}

// Edited names that differ from the stored ones, trimmed as they are saved
QHash<int, QString> NamesController::pendingSources() const {
    QHash<int, QString> out;
    const std::shared_ptr<State> st = state();
    if (!st->sourcesSupported) return out;
    for (auto it = st->drafts.cbegin(); it != st->drafts.cend(); ++it) {
        const QString trimmed = it.value().trimmed();
        if (trimmed != st->stored.value(it.key())) out.insert(it.key(), trimmed);
    }
    return out;
}

const QString *NamesController::pendingDeviceName() const {
    const std::shared_ptr<State> st = state();
    if (!st->deviceSupported || !st->deviceDraftSet || st->deviceDraft.trimmed() == st->storedDevice) return nullptr;
    return &st->deviceDraft;
}

bool NamesController::hasPending() const { return !pendingSources().isEmpty() || pendingDeviceName(); }

void NamesController::save(std::function<void(const QStringList &)> done) {
    const QHash<int, QString> sources = pendingSources();
    const std::shared_ptr<State> st = state();
    const QPointer<DeviceSession> session = m_ctx.activeSession();
    const QHash<int, QString> sentDrafts = st->drafts;
    const bool sendDevice = pendingDeviceName() != nullptr;
    const QString device = st->deviceDraft.trimmed();
    const QString sentDevice = st->deviceDraft;
    QList<int> order = sources.keys();
    std::sort(order.begin(), order.end());
    // Back to back, so the device can usually store them in one flash pass
    QVector<QPair<int, QByteArray>> jobs;
    for (int s : order) jobs.append({ s, sources.value(s).toUtf8() });
    auto refused = std::make_shared<QList<int>>();   // sources, -1 the device's
    m_ctx.device->write(
        QStringLiteral("names"),
        [jobs, sendDevice, deviceUtf8 = device.toUtf8(), refused](FfiCore *c) {
            for (const auto &job : jobs)
                if (!dspi_set_input_source_name(c, uint8_t(job.first), job.second.constData())) refused->append(job.first);
            if (sendDevice && !dspi_set_device_name(c, deviceUtf8.constData())) refused->append(-1);
            return refused->isEmpty();
        },
        [this, st, session, sentDrafts, sendDevice, sentDevice, refused, done](bool) {
            // Keep a field typed into again while the save was on its way
            for (auto it = sentDrafts.cbegin(); it != sentDrafts.cend(); ++it)
                if (!refused->contains(it.key()) && st->drafts.value(it.key()) == it.value()) st->drafts.remove(it.key());
            if (sendDevice && !refused->contains(-1) && st->deviceDraft == sentDevice) st->deviceDraftSet = false;
            QStringList failures;
            for (int s : std::as_const(*refused))
                failures << (s < 0 ? tr("the device name") : tr("the name of %1").arg(builtInTitle(s)));
            if (session) readNames(session, st);
            done(failures);
        });
}

void NamesController::revert() {
    const std::shared_ptr<State> st = state();
    st->drafts.clear();
    st->deviceDraftSet = false;
    touched();
}

void NamesController::touched() {
    emit changed();
    if (auto *hw = Registrar::instance<HardwareController>()) hw->participantChanged();
}
