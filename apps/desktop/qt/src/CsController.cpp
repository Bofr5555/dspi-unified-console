#include "CsController.h"
#include "DSPiBridge.h"

#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>

extern "C" {
#include "dspi_core.h"
}

CsController::CsController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge)
{
    // Polls bypass apply(): they must not wipe the reason of a refusal a page
    // is still showing, nor log one line per tick while the read fails.
    const auto poll = [this](const QByteArray &op) {
        if (!m_bridge->connected()) return;
        call(op);
        reload();
    };
    m_displayPoll.setInterval(1000);
    connect(&m_displayPoll, &QTimer::timeout, this, [poll]() { poll(R"({"op":"displayStatus"})"); });
    // A running macro pushes nothing, so follow it until it ends (with a ceiling:
    // steps can carry minute-long delays)
    m_macroPoll.setInterval(250);
    connect(&m_macroPoll, &QTimer::timeout, this, [this, poll]() {
        poll(R"({"op":"refreshStatus"})");
        const int running = m_model.value("ext").toMap().value("macroRunning").toInt();
        if (running == 0xFF || ++m_macroPolls >= 120) m_macroPoll.stop();
    });

    connect(m_bridge, &DSPiBridge::devicesChanged, this, &CsController::onDeviceChanged);
    connect(m_bridge, &DSPiBridge::csNotified, this, &CsController::onNotified);
    onDeviceChanged();
}

bool CsController::unsaved() const {
    return m_model.value("status").toMap().value("dirty").toBool();
}

void CsController::setWatchDisplay(bool on) {
    if (on == m_displayPoll.isActive()) return;
    if (on && supported()) m_displayPoll.start(); else m_displayPoll.stop();
    emit watchDisplayChanged();
}

// A new device (or none): read its configuration once the window has drawn
void CsController::onDeviceChanged() {
    // -- WP QD6 -- Read-only firmware shows its surfaces too; the pages' controls are off
    const bool usable = m_bridge->connected();
    const QString serial = usable ? m_bridge->selectedSerial() : QString();
    if (serial == m_serial) return;
    m_serial = serial;
    m_macroPoll.stop();
    m_displayPoll.stop();
    m_model.clear();
    m_snapshot.clear();
    m_displayState.clear();
    m_auxLive.clear();
    m_lastError.clear();
    m_revision++;
    emit changed();
    emit displayStateChanged();
    emit auxLiveChanged();
    emit reloaded();
    if (!usable) return;
    QTimer::singleShot(0, this, [this, serial]() {
        if (serial == m_serial) refresh();
    });
}

void CsController::refresh() {
    if (!m_bridge->connected()) return;
    if (!dspi_cs_fetch(m_bridge->core())) {
        // USB trouble rather than missing support: read again on the next change
        m_serial.clear();
        return;
    }
    reload();
    emit reloaded();
}

// Re-read the core's snapshot; tell the pages only if something changed
void CsController::reload() {
    QByteArray buf(64 * 1024, '\0');
    uint32_t needed = dspi_cs_snapshot(m_bridge->core(), buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) {
        buf.resize(int(needed));
        needed = dspi_cs_snapshot(m_bridge->core(), buf.data(), uint32_t(buf.size()));
    }
    buf.truncate(int(qMax<uint32_t>(1, needed) - 1));
    QJsonObject obj = QJsonDocument::fromJson(buf).object();
    QJsonObject display = obj.value("display").toObject();
    const QVariantMap state = display.take("status").toObject().toVariantMap();
    if (!display.isEmpty()) obj["display"] = display;
    if (state != m_displayState) {
        m_displayState = state;
        emit displayStateChanged();
    }
    const QVariantMap aux { { "state", obj.take("auxState").toVariant() }, { "level", obj.take("auxLevel").toVariant() } };
    if (aux != m_auxLive) {
        m_auxLive = aux;
        emit auxLiveChanged();
    }
    const QByteArray snapshot = QJsonDocument(obj).toJson(QJsonDocument::Compact);
    if (snapshot == m_snapshot) return;
    m_snapshot = snapshot;
    m_model = obj.toVariantMap();
    m_revision++;
    emit changed();
}

QVariantMap CsController::call(const QByteArray &json) {
    QByteArray buf(4096, '\0');
    uint32_t needed = dspi_cs_apply(m_bridge->core(), json.constData(), buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) return { { "ok", false }, { "error", tr("Reply too long.") } };
    buf.truncate(int(qMax<uint32_t>(1, needed) - 1));
    return QJsonDocument::fromJson(buf).object().toVariantMap();
}

QVariantMap CsController::apply(const QVariantMap &op) {
    m_lastError.clear();
    if (!m_bridge->connected()) {
        // The core is not asked, so the reason is given here
        m_lastError = tr("The device is not connected");
        return { { "ok", false }, { "status", 0xFF }, { "error", m_lastError } };
    }
    QVariantMap r = call(QJsonDocument(QJsonObject::fromVariantMap(op)).toJson(QJsonDocument::Compact));
    // The reason a reply without a device status gives; statusMessage() shows it
    m_lastError = r.value("error").toString();
    if (!m_lastError.isEmpty()) qDebug().noquote() << "Control surfaces:" << op.value("op").toString() << m_lastError;
    reload();
    const QString kind = op.value("op").toString();
    if (kind == "macroFire" && r.value("ok").toBool()) {
        m_macroPolls = 0;
        m_macroPoll.start();
    }
    return r;
}

void CsController::sendOnly(const QVariantMap &op) {
    if (m_bridge->connected())
        call(QJsonDocument(QJsonObject::fromVariantMap(op)).toJson(QJsonDocument::Compact));
}

int CsController::save() {
    const int st = apply({ { "op", "save" } }).value("status", 0xFF).toInt();
    return st;
}

int CsController::revert() {
    const int st = apply({ { "op", "revert" } }).value("status", 0xFF).toInt();
    emit reloaded();
    return st;
}

void CsController::onNotified(bool aux, bool learn) {
    if (aux) reload();
    if (learn) {
        reload();
        const QVariantMap l = m_model.value("learn").toMap();
        if (!l.isEmpty())
            emit learnFinished(l.value("state").toInt(), l.value("protocol").toInt(), l.value("code").toDouble());
    }
}

// Ask with a buffer, and again with one of the size the core says it needs
template <typename Call>
static QByteArray askCore(Call call) {
    QByteArray buf(16 * 1024, '\0');
    uint32_t needed = call(buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) {
        buf.resize(int(needed));
        needed = call(buf.data(), uint32_t(buf.size()));
    }
    buf.truncate(int(qMax<uint32_t>(1, needed) - 1));
    return buf;
}

QString CsController::statusMessage(int code) const {
    // 0xFF is "no device status": the last apply()'s own reason, when it gave one
    if (code == 0xFF && !m_lastError.isEmpty()) return m_lastError;
    if (code == 0xFF) return tr("Failed to apply the binding");
    return QString::fromUtf8(askCore([&](char *buf, uint32_t len) {
        return dspi_cs_status_text(m_bridge->core(), uint8_t(code), buf, len);
    }));
}

// -- WP CG20 --
static QByteArray compact(const QVariantMap &map) {
    return QJsonDocument(QJsonObject::fromVariantMap(map)).toJson(QJsonDocument::Compact);
}

static QVariantMap parsed(const QByteArray &json) {
    return QJsonDocument::fromJson(json).object().toVariantMap();
}

QVariantMap CsController::describe(const QString &kind, int slot, const QVariantMap &draft) const {
    const QByteArray k = kind.toUtf8(), d = compact(draft);
    return parsed(askCore([&](char *buf, uint32_t len) {
        return dspi_cs_describe(m_bridge->core(), k.constData(), uint32_t(qMax(0, slot)), d.constData(), buf, len);
    }));
}

QVariantMap CsController::edit(const QString &kind, int slot, const QVariantMap &draft,
                               const QString &id, const QVariant &value) const {
    const QByteArray k = kind.toUtf8(), d = compact(draft);
    const QByteArray c = compact({ { "id", id }, { "value", value } });
    return parsed(askCore([&](char *buf, uint32_t len) {
        return dspi_cs_edit(m_bridge->core(), k.constData(), uint32_t(qMax(0, slot)), d.constData(), c.constData(), buf, len);
    }));
}

QVariantMap CsController::newBinding(int slot, int type) const {
    return parsed(askCore([&](char *buf, uint32_t len) {
        return dspi_cs_new_binding(m_bridge->core(), uint32_t(qMax(0, slot)), uint8_t(type), buf, len);
    }));
}

QVariantMap CsController::newRecord(const QString &kind) const {
    const QByteArray k = kind.toUtf8();
    return parsed(askCore([&](char *buf, uint32_t len) {
        return dspi_cs_new_record(m_bridge->core(), k.constData(), buf, len);
    }));
}
