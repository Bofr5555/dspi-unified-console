#include "TokenStore.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QStandardPaths>

#ifdef Q_OS_LINUX
#include <QCoreApplication>
#include <QDBusArgument>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusObjectPath>
#include <QDBusVariant>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QMap>
#include <QTimer>
#include <mutex>
#endif

#ifdef Q_OS_WIN
#include <windows.h>
#include <wincred.h>
#endif

namespace {
using Outcome = TokenBackend::Outcome;

QString normalId(const QString &hubId) {
    return hubId.trimmed().toLower();
}

// ── The file ──

class FileBackend : public TokenBackend {
public:
    explicit FileBackend(const QString &path) : m_path(path) {}

    Outcome read(const QString &hubId, QString *token) override {
        QJsonObject all;
        if (!readAll(&all)) return Outcome::Failed;
        const QJsonValue v = all.value(hubId);
        if (!v.isString()) return Outcome::Missing;
        *token = v.toString();
        return Outcome::Done;
    }

    Outcome write(const QString &hubId, const QString &token) override {
        QJsonObject all;
        // A damaged file holds nothing to keep; one unreadable for now does
        if (!readAll(&all) && QFile::exists(m_path) && !QFileInfo(m_path).isReadable()) return Outcome::Failed;
        all.insert(hubId, token);
        return writeAll(all);
    }

    Outcome remove(const QString &hubId) override {
        QJsonObject all;
        if (!readAll(&all)) return Outcome::Failed;
        if (!all.contains(hubId)) return Outcome::Done;
        all.remove(hubId);
        return writeAll(all);
    }

private:
    bool readAll(QJsonObject *out) {
        QFile f(m_path);
        if (!f.exists()) return true;
        if (!f.open(QIODevice::ReadOnly)) return false;
        const QJsonDocument doc = QJsonDocument::fromJson(f.readAll());
        if (!doc.isObject()) {
            qWarning() << "DSPi Link: the token file is damaged:" << m_path;
            return false;
        }
        *out = doc.object();
        return true;
    }

    Outcome writeAll(const QJsonObject &all) {
        QDir().mkpath(QFileInfo(m_path).absolutePath());
        QSaveFile f(m_path);
        if (!f.open(QIODevice::WriteOnly)) return Outcome::Failed;
#ifdef Q_OS_UNIX
        // Before a byte of it is written. On Windows the profile's ACLs do this.
        if (!f.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner)) {
            f.cancelWriting();
            return Outcome::Failed;
        }
#endif
        if (f.write(QJsonDocument(all).toJson(QJsonDocument::Compact)) < 0 || !f.commit()) return Outcome::Failed;
        return Outcome::Done;
    }

    QString m_path;
};

#ifdef Q_OS_WIN
// ── Credential Manager ──

class CredentialManagerBackend : public TokenBackend {
public:
    Outcome read(const QString &hubId, QString *token) override {
        PCREDENTIALW cred = nullptr;
        if (!CredReadW(target(hubId).c_str(), CRED_TYPE_GENERIC, 0, &cred))
            return GetLastError() == ERROR_NOT_FOUND ? Outcome::Missing : Outcome::Failed;
        *token = QString::fromUtf8(reinterpret_cast<const char *>(cred->CredentialBlob), int(cred->CredentialBlobSize));
        CredFree(cred);
        return Outcome::Done;
    }

    Outcome write(const QString &hubId, const QString &token) override {
        std::wstring name = target(hubId);
        QByteArray secret = token.toUtf8();
        std::wstring comment = L"DSPi uConsole: DSPi Link hub token";
        CREDENTIALW cred = {};
        cred.Type = CRED_TYPE_GENERIC;
        cred.TargetName = name.data();
        cred.Comment = comment.data();
        cred.CredentialBlobSize = DWORD(secret.size());
        cred.CredentialBlob = reinterpret_cast<LPBYTE>(secret.data());
        cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
        return CredWriteW(&cred, 0) ? Outcome::Done : Outcome::Failed;
    }

    Outcome remove(const QString &hubId) override {
        if (CredDeleteW(target(hubId).c_str(), CRED_TYPE_GENERIC, 0)) return Outcome::Done;
        return GetLastError() == ERROR_NOT_FOUND ? Outcome::Done : Outcome::Failed;
    }

private:
    static std::wstring target(const QString &hubId) {
        return (QStringLiteral("DSPiConsole/link/") + hubId).toStdWString();
    }
};
#endif

#ifdef Q_OS_LINUX
// ── The Secret Service (org.freedesktop.Secret, session algorithm "plain": the
// session bus is this user's, as in every client that does not encrypt it) ──

const char kService[] = "org.freedesktop.Secret.Service";
const char kItem[] = "org.freedesktop.Secret.Item";
const char kCollection[] = "org.freedesktop.Secret.Collection";
const char kPrompt[] = "org.freedesktop.Secret.Prompt";
const char kSession[] = "org.freedesktop.Secret.Session";
const char kServicePath[] = "/org/freedesktop/secrets";
const char kDefaultCollection[] = "/org/freedesktop/secrets/aliases/default";
constexpr int kCallMs = 5000;

using StringMap = QMap<QString, QString>;

struct SecretValue {
    QDBusObjectPath session;
    QByteArray parameters;
    QByteArray value;
    QString contentType;
};

QDBusArgument &operator<<(QDBusArgument &a, const SecretValue &s) {
    a.beginStructure();
    a << s.session << s.parameters << s.value << s.contentType;
    a.endStructure();
    return a;
}

const QDBusArgument &operator>>(const QDBusArgument &a, SecretValue &s) {
    a.beginStructure();
    a >> s.session >> s.parameters >> s.value >> s.contentType;
    a.endStructure();
    return a;
}
} // namespace
Q_DECLARE_METATYPE(SecretValue)
namespace {

void registerTypes() {
    static std::once_flag once;
    std::call_once(once, [] {
        qDBusRegisterMetaType<SecretValue>();
        qDBusRegisterMetaType<StringMap>();
    });
}
} // namespace

// Waits out a Prompt's Completed signal (a slot, so it needs to be a QObject)
class SecretPromptWait : public QObject {
    Q_OBJECT
public:
    bool finished = false;
    bool dismissed = true;
    QEventLoop loop;
public slots:
    void completed(bool wasDismissed, const QDBusVariant &) {
        finished = true;
        dismissed = wasDismissed;
        loop.quit();
    }
};

namespace {

class SecretServiceBackend : public TokenBackend {
public:
    SecretServiceBackend(const QDBusConnection &bus, const QString &service, std::atomic<bool> *cancelled,
                         int promptWaitMs)
        : m_bus(bus), m_service(service), m_cancelled(cancelled), m_promptWaitMs(promptWaitMs) {
        registerTypes();
    }

    Outcome read(const QString &hubId, QString *token) override {
        Session session(*this);
        if (!session.ok()) return Outcome::Failed;
        QString item;
        const Outcome found = findUnlocked(hubId, &item);
        if (found != Outcome::Done) return found;
        const QDBusMessage reply = call(item, kItem, "GetSecret", { QVariant::fromValue(session.path()) });
        if (!isReply(reply, 1)) return Outcome::Failed;
        const auto secret = qdbus_cast<SecretValue>(reply.arguments().at(0));
        *token = QString::fromUtf8(secret.value);
        return Outcome::Done;
    }

    Outcome write(const QString &hubId, const QString &token) override {
        Session session(*this);
        if (!session.ok()) return Outcome::Failed;
        if (!unlock({ QDBusObjectPath(kDefaultCollection) })) return Outcome::Failed;

        QVariantMap properties;
        properties.insert(QStringLiteral("org.freedesktop.Secret.Item.Label"),
                          QStringLiteral("DSPi Link hub token (%1)").arg(hubId));
        properties.insert(QStringLiteral("org.freedesktop.Secret.Item.Attributes"),
                          QVariant::fromValue(attributes(hubId)));
        SecretValue secret { session.path(), QByteArray(), token.toUtf8(), QStringLiteral("text/plain") };
        const QDBusMessage reply = call(kDefaultCollection, kCollection, "CreateItem",
                                        { properties, QVariant::fromValue(secret), true });
        if (!isReply(reply, 2)) return Outcome::Failed;
        return runPrompt(pathOf(reply.arguments().at(1))) ? Outcome::Done : Outcome::Failed;
    }

    Outcome remove(const QString &hubId) override {
        QString item;
        for (;;) {
            const Outcome found = findUnlocked(hubId, &item);
            if (found == Outcome::Missing) return Outcome::Done;
            if (found == Outcome::Failed) return Outcome::Failed;
            const QDBusMessage reply = call(item, kItem, "Delete", {});
            if (!isReply(reply, 1) || !runPrompt(pathOf(reply.arguments().at(0)))) return Outcome::Failed;
            if (m_cancelled && m_cancelled->load()) return Outcome::Failed;
        }
    }

private:
    class Session {
    public:
        explicit Session(SecretServiceBackend &owner) : m_owner(owner) {
            const QDBusMessage reply = owner.call(
                kServicePath, kService, "OpenSession",
                { QStringLiteral("plain"), QVariant::fromValue(QDBusVariant(QString())) });
            if (isReply(reply, 2)) m_path = pathOf(reply.arguments().at(1));
        }
        ~Session() {
            if (ok()) m_owner.call(m_path, kSession, "Close", {});
        }
        bool ok() const { return !m_path.isEmpty(); }
        QDBusObjectPath path() const { return QDBusObjectPath(m_path); }

    private:
        SecretServiceBackend &m_owner;
        QString m_path;
    };

    static bool isReply(const QDBusMessage &reply, int arguments) {
        return reply.type() == QDBusMessage::ReplyMessage && reply.arguments().size() >= arguments;
    }

    static QString pathOf(const QVariant &v) {
        return v.value<QDBusObjectPath>().path();
    }

    static StringMap attributes(const QString &hubId) {
        StringMap map;
        map.insert(QStringLiteral("application"), QStringLiteral("io.github.weeblabs.DSPiConsole"));
        map.insert(QStringLiteral("dspi-link-hub-id"), hubId);
        return map;
    }

    QDBusMessage call(const QString &path, const char *interface, const char *method, const QVariantList &args) {
        QDBusMessage message = QDBusMessage::createMethodCall(m_service, path, QString::fromLatin1(interface),
                                                              QString::fromLatin1(method));
        message.setArguments(args);
        return m_bus.call(message, QDBus::Block, kCallMs);
    }

    // The one item for this hub, unlocking it if a keyring holds it locked
    Outcome findUnlocked(const QString &hubId, QString *item) {
        const QDBusMessage reply = call(kServicePath, kService, "SearchItems", { QVariant::fromValue(attributes(hubId)) });
        if (!isReply(reply, 2)) return Outcome::Failed;
        const auto open = qdbus_cast<QList<QDBusObjectPath>>(reply.arguments().at(0));
        if (!open.isEmpty()) {
            *item = open.first().path();
            return Outcome::Done;
        }
        const auto locked = qdbus_cast<QList<QDBusObjectPath>>(reply.arguments().at(1));
        if (locked.isEmpty()) return Outcome::Missing;
        if (!unlock({ locked.first() })) return Outcome::Failed;
        *item = locked.first().path();
        return Outcome::Done;
    }

    bool unlock(const QList<QDBusObjectPath> &objects) {
        const QDBusMessage reply = call(kServicePath, kService, "Unlock", { QVariant::fromValue(objects) });
        return isReply(reply, 2) && runPrompt(pathOf(reply.arguments().at(1)));
    }

    // "/" means no prompt is needed. False: dismissed, no answer, or cancelled.
    bool runPrompt(const QString &path) {
        if (path.isEmpty()) return false;
        if (path == QLatin1String("/")) return true;
        SecretPromptWait wait;
        if (!m_bus.connect(m_service, path, QString::fromLatin1(kPrompt), QStringLiteral("Completed"), &wait,
                           SLOT(completed(bool, QDBusVariant))))
            return false;
        const QDBusMessage shown = call(path, kPrompt, "Prompt", { QString() });
        bool ok = false;
        if (shown.type() == QDBusMessage::ReplyMessage) {
            QElapsedTimer waited;
            waited.start();
            QTimer tick;
            QObject::connect(&tick, &QTimer::timeout, &wait, [&] {
                if ((m_cancelled && m_cancelled->load()) || waited.elapsed() > m_promptWaitMs) wait.loop.quit();
            });
            tick.start(100);
            if (!wait.finished) wait.loop.exec();
            ok = wait.finished && !wait.dismissed;
        }
        m_bus.disconnect(m_service, path, QString::fromLatin1(kPrompt), QStringLiteral("Completed"), &wait,
                         SLOT(completed(bool, QDBusVariant)));
        return ok;
    }

    QDBusConnection m_bus;
    QString m_service;
    std::atomic<bool> *m_cancelled;
    int m_promptWaitMs;
};
#endif // Q_OS_LINUX
} // namespace

#ifdef Q_OS_LINUX
std::unique_ptr<TokenBackend> makeSecretServiceBackend(const QDBusConnection &bus, const QString &service,
                                                       std::atomic<bool> *cancelled, int promptWaitMs) {
    return std::make_unique<SecretServiceBackend>(bus, service, cancelled, promptWaitMs);
}
#endif

std::unique_ptr<TokenBackend> makeTokenFileBackend(const QString &path) {
    return std::make_unique<FileBackend>(path);
}

std::unique_ptr<TokenBackend> TokenBackend::forPlatform(std::atomic<bool> *cancelled) {
#if defined(Q_OS_WIN)
    Q_UNUSED(cancelled)
    return std::make_unique<CredentialManagerBackend>();
#elif defined(Q_OS_LINUX)
    constexpr int kPromptWaitMs = 120000;   // long enough to type a keyring's password
    return makeSecretServiceBackend(QDBusConnection::sessionBus(), QStringLiteral("org.freedesktop.secrets"),
                                    cancelled, kPromptWaitMs);
#else
    Q_UNUSED(cancelled)
    return nullptr;
#endif
}

// ── The store ──

TokenStore::TokenStore(std::unique_ptr<TokenBackend> system, const QString &filePath)
    : m_system(std::move(system)), m_file(makeTokenFileBackend(filePath)), m_filePath(filePath) {}

TokenStore::~TokenStore() = default;

std::unique_ptr<TokenStore> TokenStore::forThisUser(std::atomic<bool> *cancelled) {
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation);
    return std::make_unique<TokenStore>(TokenBackend::forPlatform(cancelled), dir + QStringLiteral("/link-tokens.json"));
}

QString TokenStore::filePath() const {
    return m_filePath;
}

QString TokenStore::load(const QString &hubId) {
    const QString id = normalId(hubId);
    if (id.isEmpty()) return QString();
    QString token;
    if (m_system && m_system->read(id, &token) == Outcome::Done) return token;
    if (m_file->read(id, &token) != Outcome::Done) return QString();
    // A file copy from when the secret store was away
    if (m_system && m_system->write(id, token) == Outcome::Done) m_file->remove(id);
    return token;
}

TokenStore::Saved TokenStore::save(const QString &hubId, const QString &token) {
    const QString id = normalId(hubId);
    if (id.isEmpty() || token.isEmpty()) return Saved::Nowhere;
    if (m_system && m_system->write(id, token) == Outcome::Done) {
        m_file->remove(id);
        return Saved::InSecretStore;
    }
    if (m_file->write(id, token) == Outcome::Done) return Saved::InFile;
    qWarning() << "DSPi Link: a hub's token could not be stored anywhere";
    return Saved::Nowhere;
}

void TokenStore::forget(const QString &hubId) {
    const QString id = normalId(hubId);
    if (id.isEmpty()) return;
    if (m_system) m_system->remove(id);
    m_file->remove(id);
}

#ifdef Q_OS_LINUX
#include "TokenStore.moc"
#endif
