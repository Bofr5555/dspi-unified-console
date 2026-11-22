#include "LinkController.h"

#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "Poller.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "TokenStore.h"

#include "AppIdentity.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonObject>
#include <QPointer>

static const Registrar::Controller<LinkController> linkController;
// The banners of a Link session, under the firmware banners and above the demo's
static const Registrar::Banner linkBanner("LinkBanner", 20);
static const Registrar::Banner lockNotice("LockNotice", 25);
static const Registrar::Banner tokenNotice("TokenNotice", 40);
// -- WP SECQT -- a new hub key, once, beside the token notice
static const Registrar::Banner keyNoticeBanner("KeyNotice", 41);

LinkController::LinkController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_registry(ctx.registry), m_poller(ctx.poller) {
    for (DeviceSession *s : m_registry->sessions()) addSession(s);
    connect(m_registry, &DeviceRegistry::sessionOpened, this, &LinkController::addSession);
    connect(m_registry, &DeviceRegistry::sessionClosing, this, &LinkController::dropSession);
    // Choosing a device ends a dial for another one
    connect(m_registry, &DeviceRegistry::activeChanged, this, [this] {
        leaveLobby();
        showSubject();
    });
    showSubject();
    // Before the core goes (main() frees it after the event loop ends)
    connect(qApp, &QCoreApplication::aboutToQuit, this, &LinkController::shutdown);
}

LinkController::LinkController(FfiCore *core, std::unique_ptr<TokenStore> store, QObject *parent)
    : QObject(parent), m_subject(new LinkSession(core, std::move(store), this)) {
    m_links.insert(nullptr, m_subject);
    showSubject();
}

LinkController::~LinkController() { shutdown(); }

LinkController *LinkController::create(QQmlEngine *, QJSEngine *) {
    LinkController *self = Registrar::instance<LinkController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void LinkController::shutdown() {
    if (m_shutDown) return;
    m_shutDown = true;
    for (LinkSession *l : std::as_const(m_links)) l->shutdown();
    if (m_lobby) m_lobby->shutdown();
    if (m_lobbyCore) dspi_core_free(m_lobbyCore);
    m_lobbyCore = nullptr;
}

// ---- Which session -------------------------------------------------------------

void LinkController::addSession(DeviceSession *session) {
    if (m_links.contains(session) || m_shutDown) return;
    auto *link = new LinkSession(session->handle(), session->model(), this);
    m_links.insert(session, link);
    // A token is stored for the hub, whichever device's session got it
    connect(link, &LinkSession::tokensChanged, this, &LinkController::tokensChanged);
}

void LinkController::dropSession(DeviceSession *session) {
    if (session->isMain()) {
        // Its device is closed, not the session: nothing reopens
        if (LinkSession *l = m_links.value(session); l && l->inSession()) l->leaveForLocal();
        return;
    }
    LinkSession *link = m_links.take(session);
    if (!link) return;
    if (link == m_subject) showSubject();
    link->shutdown();
    link->deleteLater();
}

void LinkController::showSubject() {
    LinkSession *now = m_inLobby ? m_lobby : m_registry ? m_links.value(m_registry->active()) : m_subject;
    if (!now) return;
    if (m_subject) disconnect(m_subject, nullptr, this, nullptr);
    m_subject = now;
    connect(now, &LinkSession::statusChanged, this, &LinkController::statusChanged);
    connect(now, &LinkSession::busyChanged, this, &LinkController::busyChanged);
    connect(now, &LinkSession::hubChanged, this, &LinkController::hubChanged);
    connect(now, &LinkSession::devicesChanged, this, &LinkController::devicesChanged);
    connect(now, &LinkSession::errorChanged, this, &LinkController::errorChanged);
    connect(now, &LinkSession::tokenWarningChanged, this, &LinkController::tokenWarningChanged);
    connect(now, &LinkSession::securityChanged, this, &LinkController::securityChanged);
    connect(now, &LinkSession::pairLinkChanged, this, &LinkController::pairLinkChanged);
    connect(now, &LinkSession::keyNoticeChanged, this, &LinkController::keyNoticeChanged);
    // Over a hub every poll runs slower (Poller)
    connect(now, &LinkSession::deviceBound, this, [this](const QString &serial) {
        if (m_poller && m_subject != m_lobby) m_poller->setSlowTransport(true);
        emit deviceBound(serial);
    });
    connect(now, &LinkSession::sessionEnded, this, [this] {
        if (m_poller && m_subject != m_lobby) m_poller->setSlowTransport(false);
        emit sessionEnded();
    });
    connect(now, &LinkSession::reconnected, this, &LinkController::reconnected);
    connect(now, &LinkSession::dialFailed, this, &LinkController::dialFailed);
    // Not every session's: the rest of each is connected in addSession
    for (LinkSession *l : std::as_const(m_links))
        connect(l, &LinkSession::tokensChanged, this, &LinkController::tokensChanged, Qt::UniqueConnection);
    if (m_lobby) connect(m_lobby, &LinkSession::tokensChanged, this, &LinkController::tokensChanged, Qt::UniqueConnection);
    emit subjectChanged();
    emit statusChanged();
    emit busyChanged();
    emit hubChanged();
    emit devicesChanged();
    emit errorChanged();
    emit tokenWarningChanged();
    emit securityChanged();
    emit pairLinkChanged();
    emit keyNoticeChanged();
}

// Device tabs: the dial has a core of its own, so the open device and its
// session (a hub's unit has one) stay as they are
LinkSession *LinkController::dialer() {
    if (!m_registry || !m_registry->keepOpen() || m_shutDown) return m_subject;
    if (!m_lobby) {
        m_lobbyCore = dspi_core_new();
        if (!m_lobbyCore) return m_subject;
        m_lobby = new LinkSession(m_lobbyCore, std::unique_ptr<TokenStore>(), this);
    }
    if (!m_inLobby) {
        m_inLobby = true;
        showSubject();
    }
    return m_lobby;
}

void LinkController::leaveLobby() {
    if (!m_inLobby) return;
    m_inLobby = false;
    m_lobby->closeSession();
}

// The unit on its own core, through the hub shown here
void LinkController::openBeside(const QString &serial) {
    LinkSession *through = m_subject;
    const QJsonObject target{
        { QStringLiteral("kind"), QStringLiteral("hub") },
        { QStringLiteral("url"), through->sessionUrl() },
        { QStringLiteral("hubId"), through->hubId() },
        { QStringLiteral("serial"), serial },
        { QStringLiteral("clientName"), LinkSession::clientName() },
        { QStringLiteral("appVersion"), AppIdentity::version() },
        { QStringLiteral("credential"), through->sessionCredential() },
        { QStringLiteral("tlsAdvertised"), through->sessionTls() },
    };
    QPointer<LinkSession> dialled(through);
    m_registry->open(
        target,
        [this, dialled, serial](DeviceSession *session, const QString &code, const QString &error) {
            if (!session) {
                if (dialled) dialled->setError(code, error);
                return;
            }
            addSession(session);
            if (dialled) m_links.value(session)->adoptBound(*dialled, serial);
            m_registry->activate(session);   // leaves the lobby
        },
        false);
}

// ---- Calls ---------------------------------------------------------------------

void LinkController::connectToHub(const QString &url, const QString &expectedHubId, bool tlsAdvertised) {
    dialer()->connectToHub(url, expectedHubId, tlsAdvertised);
}

bool LinkController::connectToAddress(const QString &address, const QString &expectedHubId) {
    return dialer()->connectToAddress(address, expectedHubId);
}

bool LinkController::openPairLink(const QString &text) { return dialer()->openPairLink(text); }

void LinkController::pair(const QString &pin) { m_subject->pair(pin); }

void LinkController::bindDevice(int handle, const QString &serial) {
    if (m_registry && m_registry->keepOpen() && (m_inLobby || !m_subject->boundSerial().isEmpty())) openBeside(serial);
    else m_subject->bindDevice(handle, serial);
}

void LinkController::closeSession() {
    if (m_inLobby) {
        leaveLobby();
        showSubject();
        return;
    }
    // A device opened beside the others goes with its hub session
    if (m_registry && !m_registry->active()->isMain() && !m_subject->boundSerial().isEmpty()) {
        m_registry->close(m_registry->activeId());
        return;
    }
    m_subject->closeSession();
}

void LinkController::leaveForLocal() {
    if (m_inLobby) {
        leaveLobby();
        showSubject();
    }
    m_subject->leaveForLocal();
}

void LinkController::retryNow() { m_subject->retryNow(); }
void LinkController::dismissTokenWarning() { m_subject->dismissTokenWarning(); }
void LinkController::rePair() { m_subject->rePair(); }
void LinkController::dismissRePair() { m_subject->dismissRePair(); }
void LinkController::dismissKeyNotice() { m_subject->dismissKeyNotice(); }
void LinkController::refreshStatus() { m_subject->refreshStatus(); }
void LinkController::setStaleDialHook(std::function<void()> hook) { m_subject->setStaleDialHook(std::move(hook)); }

// ---- What it shows -------------------------------------------------------------

QString LinkController::phase() const { return m_subject->phase(); }
QString LinkController::phaseKey() const { return m_subject->phaseKey(); }
QString LinkController::phaseText() const { return m_subject->phaseText(); }
QString LinkController::actionKey() const { return m_subject->actionKey(); }
QString LinkController::actionText() const { return m_subject->actionText(); }
int LinkController::reconnectAttempt() const { return m_subject->reconnectAttempt(); }
int LinkController::reconnectInMs() const { return m_subject->reconnectInMs(); }
QString LinkController::pinSource() const { return m_subject->pinSource(); }
QString LinkController::closeReason() const { return m_subject->closeReason(); }
QString LinkController::role() const { return m_subject->role(); }
int LinkController::session() const { return m_subject->session(); }
bool LinkController::busy() const { return m_subject->busy(); }
QString LinkController::hubId() const { return m_subject->hubId(); }
QString LinkController::hubName() const { return m_subject->hubName(); }
QVariantList LinkController::devices() const { return m_subject->devices(); }
QString LinkController::boundSerial() const { return m_subject->boundSerial(); }
QString LinkController::errorCode() const { return m_subject->errorCode(); }
QString LinkController::errorText() const { return m_subject->errorText(); }
QString LinkController::tokenWarning() const { return m_subject->tokenWarning(); }
QString LinkController::lockNotice() const { return m_subject->lockNotice(); }
QString LinkController::sessionTip() const { return m_subject->sessionTip(); }
bool LinkController::encrypted() const { return m_subject->encrypted(); }
bool LinkController::unsecured() const { return m_subject->unsecured(); }
bool LinkController::local() const { return m_subject->local(); }
QString LinkController::pairLinkPin() const { return m_subject->pairLinkPin(); }
QString LinkController::hubKey() const { return m_subject->hubKey(); }
QString LinkController::unsecuredLabel() const { return m_subject->unsecuredLabel(); }
QString LinkController::unsecuredInfoTitle() const { return m_subject->unsecuredInfoTitle(); }
QString LinkController::unsecuredInfoText() const { return m_subject->unsecuredInfoText(); }
QString LinkController::keyNotice() const { return m_subject->keyNotice(); }
bool LinkController::rePairOffered() const { return m_subject->rePairOffered(); }
bool LinkController::inSession() const { return m_subject->inSession(); }
QString LinkController::sessionUrl() const { return m_subject->sessionUrl(); }
QString LinkController::sessionCredential() const { return m_subject->sessionCredential(); }
bool LinkController::sessionTls() const { return m_subject->sessionTls(); }
