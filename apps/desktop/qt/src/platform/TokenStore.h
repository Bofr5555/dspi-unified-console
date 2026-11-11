#ifndef TOKENSTORE_H
#define TOKENSTORE_H

#include <QString>
#include <atomic>
#include <memory>

// Where this app keeps the token each DSPi Link hub issued it, by hub id. A token
// lets a hub be used again without its PIN, so it is a secret: it is kept in the
// system's secret store (Credential Manager, the Secret Service), and only when
// that does not answer in a file only this user can read, which the caller warns
// about. Never logged.
//
// Every call may block, the Secret Service's for as long as a person takes to
// unlock a keyring: keep them off the UI thread, and on one thread.

// One place tokens can live. Credential Manager and the Secret Service are these,
// and so is the file.
class TokenBackend {
public:
    enum class Outcome {
        Done,       // read: `token` holds it. write, remove: it happened
        Missing,    // read: nothing is stored under that id
        Failed,     // this place did not answer; nothing is known
    };
    virtual ~TokenBackend() = default;
    virtual Outcome read(const QString &hubId, QString *token) = 0;
    virtual Outcome write(const QString &hubId, const QString &token) = 0;
    virtual Outcome remove(const QString &hubId) = 0;

    // This platform's secret store, null if it has none (macOS's Qt build).
    static std::unique_ptr<TokenBackend> forPlatform(std::atomic<bool> *cancelled);
};

class TokenStore {
public:
    enum class Saved { InSecretStore, InFile, Nowhere };

    // `system` may be null. The file need not exist.
    TokenStore(std::unique_ptr<TokenBackend> system, const QString &filePath);
    ~TokenStore();

    // Link-tokens.json in the app's config directory, and the platform's store.
    // `cancelled` (may be null, outlives the store) ends a wait for a person.
    static std::unique_ptr<TokenStore> forThisUser(std::atomic<bool> *cancelled);
    QString filePath() const;

    // "" when none is stored or it cannot be read. A hub id is matched without case.
    // A token found only in the file moves to the secret store once that answers.
    QString load(const QString &hubId);
    Saved save(const QString &hubId, const QString &token);
    // Call this only when a hub refuses the token (token_refused). A hub that is
    // busy, limiting tries or just someone else says nothing about the token.
    void forget(const QString &hubId);

private:
    std::unique_ptr<TokenBackend> m_system;
    std::unique_ptr<TokenBackend> m_file;
    QString m_filePath;
};

// The file: one JSON object of hub id to token, mode 0600, replaced atomically.
std::unique_ptr<TokenBackend> makeTokenFileBackend(const QString &path);

#ifdef Q_OS_LINUX
#include <QDBusConnection>
// The Secret Service over this connection. `service` is empty for a direct peer
// connection (the smoke driver's fake).
std::unique_ptr<TokenBackend> makeSecretServiceBackend(const QDBusConnection &bus, const QString &service,
                                                       std::atomic<bool> *cancelled, int promptWaitMs);
#endif

#endif
