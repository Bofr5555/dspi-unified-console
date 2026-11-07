#include "LinkController.h"
#include "DSPiBridge.h"
#include "Registrar.h"
#include "TokenStore.h"

#include <QCoreApplication>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QMetaObject>
#include <QRunnable>
#include <QSysInfo>

static const Registrar::Controller<LinkController> linkRegistered("linkSession");
static const Registrar::Banner linkBanner("qrc:/qml/components/LinkBanner.qml");

namespace {
// Connect, pair and select act on the hub, and a reply too long for this means the
// call happened: they never go again for that reason.
constexpr uint32_t kActReplyBytes = 16 * 1024;
const char kReplyTooLong[] = "reply_too_long";

QJsonObject parsed(const QByteArray &bytes) {
    const QJsonDocument doc = QJsonDocument::fromJson(bytes);
    return doc.isObject() ? doc.object() : QJsonObject();
}

QJsonObject failure(const char *code, const QString &text) {
    return QJsonObject { { QStringLiteral("ok"), false }, { QStringLiteral("code"), QString::fromLatin1(code) },
                         { QStringLiteral("error"), text } };
}

// A call that acts on the hub
template <class F>
QJsonObject actReply(F call) {
    QByteArray buf(kActReplyBytes, 0);
    const uint32_t needed = call(buf.data(), kActReplyBytes);
    if (needed == 0) return failure("core", QStringLiteral("The core did not answer."));
    if (needed > kActReplyBytes) return failure(kReplyTooLong, QStringLiteral("The hub's reply was too long to read."));
    return parsed(buf.left(int(needed) - 1));
}

// A read: safe to ask twice
template <class F>
QJsonObject readReply(F call) {
    QByteArray buf(8 * 1024, 0);
    uint32_t needed = call(buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) {
        buf.resize(int(needed));
        needed = call(buf.data(), uint32_t(buf.size()));
    }
    if (needed == 0 || needed > uint32_t(buf.size())) return QJsonObject();
    return parsed(buf.left(int(needed) - 1));
}

bool isOk(const QJsonObject &reply) {
    return reply.value(QStringLiteral("ok")).toBool();
}

QString text(const QJsonObject &o, const char *key) {
    return o.value(QLatin1String(key)).toString();
}
}

LinkController::LinkController(DSPiBridge *bridge, QObject *parent)
    : LinkController(bridge->core(), TokenStore::forThisUser(&m_closing), parent) {}

LinkController::LinkController(FfiCore *core, std::unique_ptr<TokenStore> store, QObject *parent)
    : QObject(parent), m_core(core), m_store(std::move(store))
{
    m_worker.setMaxThreadCount(1);
    dspi_link_set_listener(m_core, &LinkController::eventCallback, this);
    refreshStatus();
}

LinkController::~LinkController() {
    m_closing = true;
    m_generation++;
    // No callback runs after this, and none is left waiting on this object
    dspi_link_set_listener(m_core, nullptr, nullptr);
    dspi_link_close(m_core);
    m_worker.waitForDone();     // a dial in progress finishes first, up to its timeout
}

// Core thread, with no core lock held. Never blocks; never frees the core. The
// strings are good for this call only.
void LinkController::eventCallback(uint8_t event, int32_t code, const char *text, const char *token, void *userData) {
    auto *self = static_cast<LinkController *>(userData);
    const QString textCopy = QString::fromUtf8(text);
    switch (event) {
    case LINK_EVENT_DEVICES_CHANGED:
        QMetaObject::invokeMethod(self, [self] { self->refreshDevices(); }, Qt::QueuedConnection);
        break;
    case LINK_EVENT_CLOSED:
        Q_UNUSED(code)
        QMetaObject::invokeMethod(self, [self, textCopy] {
            self->m_closeReason = textCopy;
            self->refreshStatus();
        }, Qt::QueuedConnection);
        break;
    case LINK_EVENT_STATUS_CHANGED:
        QMetaObject::invokeMethod(self, [self] { self->refreshStatus(); }, Qt::QueuedConnection);
        break;
    case LINK_EVENT_TOKEN_ISSUED: {
        // Through the UI thread too, so it stays in order with a refusal seen there
        const QString tokenCopy = QString::fromUtf8(token);
        QMetaObject::invokeMethod(self, [self, textCopy, tokenCopy] { self->saveToken(textCopy, tokenCopy); },
                                  Qt::QueuedConnection);
        break;
    }
    default:
        break;
    }
}

void LinkController::post(unsigned generation, std::function<void()> job) {
    setBusy(1);
    m_worker.start([this, generation, job = std::move(job)] {
        if (m_generation.load() == generation) job();
        QMetaObject::invokeMethod(this, [this] { setBusy(-1); }, Qt::QueuedConnection);
    });
}

void LinkController::setBusy(int delta) {
    const bool was = busy();
    m_busy += delta;
    if (busy() != was) emit busyChanged();
}

void LinkController::setError(const QString &code, const QString &errorText) {
    if (code == m_errorCode && errorText == m_errorText) return;
    m_errorCode = code;
    m_errorText = errorText;
    emit errorChanged();
}

bool LinkController::connectToAddress(const QString &address, const QString &expectedHubId) {
    const QByteArray typed = address.toUtf8();
    const QJsonObject where = readReply([&](char *buf, uint32_t len) {
        return dspi_link_parse_address(typed.constData(), buf, len);
    });
    if (!isOk(where)) {
        setError(text(where, "code"), text(where, "error"));
        return false;
    }
    const QByteArray host = text(where, "host").toUtf8();
    const uint16_t port = uint16_t(where.value(QStringLiteral("port")).toInt());
    char url[512];
    const uint32_t needed = dspi_link_ws_url(host.constData(), port, nullptr, url, sizeof(url));
    if (needed == 0 || needed > sizeof(url)) {
        setError(QStringLiteral("invalid"), QStringLiteral("That address is too long."));
        return false;
    }
    connectToHub(QString::fromUtf8(url), expectedHubId);
    return true;
}

void LinkController::connectToHub(const QString &url, const QString &expectedHubId) {
    const unsigned generation = ++m_generation;
    setError(QString(), QString());
    m_lastSavedToken.clear();
    m_refusalHandled = false;
    m_analyserWanted = false;
    m_sessionHubId.clear();
    if (m_hubId != expectedHubId || !m_hubName.isEmpty()) {
        m_hubId = expectedHubId;
        m_hubName.clear();
        emit hubChanged();
    }
    const QByteArray urlBytes = url.toUtf8();
    const QByteArray name = QSysInfo::machineHostName().toUtf8();
    const QByteArray version = QCoreApplication::applicationVersion().toUtf8();
    post(generation, [=] {
        const auto dial = [&](const QByteArray &token, const QByteArray &hub) {
            return actReply([&](char *buf, uint32_t len) {
                return dspi_link_connect(m_core, urlBytes.constData(), name.constData(), version.constData(),
                                         token.constData(), hub.constData(), buf, len);
            });
        };
        QByteArray hub = expectedHubId.toUtf8();
        QJsonObject reply = dial(hub.isEmpty() ? QByteArray("") : m_store->load(expectedHubId).toUtf8(), hub);
        // Not known ahead: this hub saw no token. Now it has said who it is, ours
        // goes out on a new dial that holds it to that.
        if (isOk(reply) && hub.isEmpty() && reply.value(QStringLiteral("needsPairing")).toBool()
            && m_generation.load() == generation) {
            const QString found = text(reply, "hubId");
            const QString token = m_store->load(found);
            if (!token.isEmpty()) {
                hub = found.toUtf8();
                reply = dial(token.toUtf8(), hub);
            }
        }
        // Left or replaced during the dial: this session is not wanted. The next
        // call's job runs after this one.
        if (m_generation.load() != generation) {
            if (isOk(reply)) dspi_link_close(m_core);
            return;
        }
        QMetaObject::invokeMethod(this, [this, generation, reply] {
            if (m_generation.load() != generation) return;
            if (isOk(reply)) applyHello(reply);
            else setError(text(reply, "code"), text(reply, "error"));
            refreshStatus();
            refreshDevices();
        }, Qt::QueuedConnection);
    });
}

void LinkController::applyHello(const QJsonObject &hello) {
    m_hubId = text(hello, "hubId");
    m_sessionHubId = m_hubId;
    m_hubName = text(hello, "hubName");
    emit hubChanged();
}

void LinkController::pair(const QString &pin) {
    const unsigned generation = m_generation.load();
    setError(QString(), QString());
    const QByteArray pinBytes = pin.toUtf8();
    post(generation, [=] {
        const QJsonObject reply = actReply([&](char *buf, uint32_t len) {
            return dspi_link_pair(m_core, pinBytes.constData(), buf, len);
        });
        QMetaObject::invokeMethod(this, [this, generation, reply] {
            if (m_generation.load() != generation) return;
            if (isOk(reply)) {
                // The listener delivers it too, and first or not, one save is enough
                const QString token = text(reply, "token");
                if (!token.isEmpty()) saveToken(m_hubId, token);
            } else {
                setError(text(reply, "code"), text(reply, "error"));
            }
            refreshStatus();
            refreshDevices();
        }, Qt::QueuedConnection);
    });
}

void LinkController::bindDevice(int handle, const QString &serial) {
    const unsigned generation = m_generation.load();
    setError(QString(), QString());
    const QByteArray serialBytes = serial.toUtf8();
    post(generation, [=] {
        QJsonObject reply = actReply([&](char *buf, uint32_t len) {
            return dspi_link_select_device(m_core, uint32_t(handle), serialBytes.constData(), buf, len);
        });
        if (isOk(reply) && !dspi_fetch_all(m_core))
            reply = failure("transfer", QStringLiteral("Connected, but the device's settings could not be read."));
        QMetaObject::invokeMethod(this, [this, generation, reply, serial] {
            if (m_generation.load() != generation) return;
            if (isOk(reply)) emit deviceBound(serial);
            else setError(text(reply, "code"), text(reply, "error"));
            refreshStatus();
        }, Qt::QueuedConnection);
    });
}

void LinkController::closeSession() {
    // A job waiting for the worker is dropped, and a dial in progress is closed
    // when it comes back
    m_generation++;
    dspi_link_close(m_core);
    m_analyserWanted = false;
    m_sessionHubId.clear();
    if (!m_hubId.isEmpty() || !m_hubName.isEmpty()) {
        m_hubId.clear();
        m_hubName.clear();
        emit hubChanged();
    }
    setError(QString(), QString());
    refreshStatus();
    refreshDevices();
}

void LinkController::retryNow() {
    dspi_link_retry_now(m_core);
}

void LinkController::dismissTokenWarning() {
    if (m_tokenWarning.isEmpty()) return;
    m_tokenWarning.clear();
    emit tokenWarningChanged();
}

// Reads the core's state; no traffic
void LinkController::refreshStatus() {
    const QJsonObject status = readReply([this](char *buf, uint32_t len) { return dspi_link_status(m_core, buf, len); });
    if (!isOk(status)) return;
    const QString phase = text(status, "phase");
    const QString pinSource = text(status, "pinSource");
    const bool phaseChanged = phase != m_phase;
    const QString previous = m_phase;

    const QByteArray phaseBytes = phase.toUtf8(), pinBytes = pinSource.toUtf8();
    const QJsonObject wording = readReply([&](char *buf, uint32_t len) {
        return dspi_link_phase_text(phaseBytes.constData(), pinBytes.constData(), buf, len);
    });

    m_phase = phase;
    m_pinSource = pinSource;
    m_phaseKey = text(wording, "key");
    m_phaseText = text(wording, "text");
    m_actionKey = text(wording, "actionKey");
    m_actionText = text(wording, "action");
    m_reconnectAttempt = status.value(QStringLiteral("reconnectAttempt")).toInt();
    m_reconnectInMs = status.value(QStringLiteral("reconnectInMs")).toInt();
    m_role = text(status, "role");
    const QString reason = text(status, "closeReason");
    if (!reason.isEmpty()) m_closeReason = reason;
    else if (phase != QLatin1String("closed") && phase != QLatin1String("refused")) m_closeReason.clear();
    emit statusChanged();

    // The hub turned the token down, when dialled or later: this is the one case
    // that deletes it. It stays handled until the hub issues another. Only for the
    // session whose hello has come: until then this status may be the old session's.
    const bool refused = status.value(QStringLiteral("tokenRefused")).toBool();
    if (refused && !m_refusalHandled && !m_sessionHubId.isEmpty()) {
        m_refusalHandled = true;
        m_lastSavedToken.clear();
        const QString hub = m_sessionHubId;
        setBusy(1);
        m_worker.start([this, hub] {
            m_store->forget(hub);
            QMetaObject::invokeMethod(this, [this, hub] {
                setBusy(-1);
                emit tokensChanged(hub);
            }, Qt::QueuedConnection);
        });
    } else if (!refused) {
        m_refusalHandled = false;
    }

    // After a reconnect the analyser is the client's to start again, if it ran. A
    // start while it runs is a no-op, so a loss too short to see as a phase is covered.
    const bool isBound = phase == QLatin1String("bound");
    if (isBound || phase == QLatin1String("reconnecting")) {
        RtaCapsInfo caps;
        dspi_rta_get_caps(m_core, &caps);
        m_analyserWanted = m_analyserWanted || caps.supported;
    }
    if (isBound && m_analyserWanted) {
        // Takes the core lock, which a re-read holds for a while
        post(m_generation.load(), [this] { dspi_rta_start(m_core); });
    }
    if (isBound && phaseChanged && previous == QLatin1String("reconnecting")) emit reconnected();
}

void LinkController::refreshDevices() {
    const QJsonObject reply = readReply([this](char *buf, uint32_t len) { return dspi_link_devices(m_core, buf, len); });
    if (!isOk(reply)) return;
    const QVariantList devices = reply.value(QStringLiteral("value")).toArray().toVariantList();
    if (devices == m_devices) return;
    m_devices = devices;
    emit devicesChanged();
}

// Runs on the UI thread and hands the store to the worker, so saves and deletes
// stay in the order the core's events came in.
void LinkController::saveToken(const QString &hubId, const QString &token) {
    if (token.isEmpty() || token == m_lastSavedToken) return;
    m_lastSavedToken = token;
    m_refusalHandled = false;
    setBusy(1);
    m_worker.start([this, hubId, token] {
        const TokenStore::Saved saved = m_store->save(hubId, token);
        QMetaObject::invokeMethod(this, [this, hubId, saved] {
            setBusy(-1);
            QString warning;
            if (saved == TokenStore::Saved::InFile) {
                qWarning() << "DSPi Link: no secret store answered; the hub's token is in" << m_store->filePath();
                warning = QStringLiteral("No secret store answered, so the pairing with this hub is saved in a "
                                         "file only your account can read.");
            } else if (saved == TokenStore::Saved::Nowhere) {
                warning = QStringLiteral("The pairing with this hub could not be saved. It will ask for a PIN next time.");
            }
            if (warning != m_tokenWarning) {
                m_tokenWarning = warning;
                emit tokenWarningChanged();
            }
            emit tokensChanged(hubId);
        }, Qt::QueuedConnection);
    });
}
