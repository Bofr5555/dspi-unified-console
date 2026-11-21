#ifndef APPUPDATER_H
#define APPUPDATER_H

#include <QDateTime>
#include <QScopedPointer>
#include <QObject>
#include <QPointer>
#include <QUrl>
#include <QVector>
#include <QtQml/qqmlregistration.h>
#include <array>
#include <climits>

#include "Registrar.h"

class QNetworkAccessManager;
class QNetworkReply;
class QTimer;
class QQmlEngine;
class QJSEngine;

// -- WP QP17 -- Is there a newer release of this app? (behaviour.md §17, the
// Mac's AppUpdateChecker for the rules.) It only checks and links: nothing is
// downloaded or installed.
//
// - The feed is a GitHub-style release list (JSON array of {tag_name,
//   html_url, draft, name, body, assets: [{name}]}). Which one: the build's
//   DSPI_UPDATE_FEED_URL (CMake, https only, empty by default and in
//   distribution packages, which then never check), or the settings key
//   updates/feedUrl when it points at this machine (tests, a local feed).
// - Opt-in: updates/automatic is off until the user turns it on. Then the app
//   checks 5 s after launch and once a day. "Check Now" always checks.
// - A check sends what the privacy line says: the host, a User-Agent of
//   uConsole-for-DSPi/<version>, Accept, "en" as the language, and nothing stored:
//   no cookie is kept or sent.
// - A release counts when it is no draft, its page is https, its tag parses
//   as a version above this app's, and it carries a package of this app for
//   this platform (uConsole-for-DSPi-<version>-...). A final release's user is not
//   offered betas.
// - What an automatic check finds raises the banner (`announce`), unless the
//   version was skipped; "Later" hides it until a check finds one again.
//
// The Updates page (UpdatesPage.qml) reads the properties below through
// SettingsController::setUpdater; the banner is UpdateBanner.qml.
class AppUpdater : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool feedAvailable READ hasFeed NOTIFY feedChanged)
    Q_PROPERTY(QString feedHost READ feedHost NOTIFY feedChanged)
    Q_PROPERTY(QString userAgent READ userAgentText NOTIFY feedChanged)
    Q_PROPERTY(bool automatic READ automatic WRITE setAutomatic NOTIFY automaticChanged)
    Q_PROPERTY(bool checking READ checking NOTIFY stateChanged)
    Q_PROPERTY(QDateTime lastChecked READ lastChecked NOTIFY stateChanged)
    Q_PROPERTY(QString currentVersion READ currentVersion NOTIFY feedChanged)
    // "" before any check, "current", "available" or "failed"
    Q_PROPERTY(QString result READ result NOTIFY stateChanged)
    Q_PROPERTY(bool available READ available NOTIFY stateChanged)
    Q_PROPERTY(QString releaseName READ releaseName NOTIFY stateChanged)
    Q_PROPERTY(QString releaseVersion READ releaseVersion NOTIFY stateChanged)
    Q_PROPERTY(QString releaseNotes READ releaseNotes NOTIFY stateChanged)
    Q_PROPERTY(QString releaseUrl READ releaseUrl NOTIFY stateChanged)
    Q_PROPERTY(bool firmwareIncluded READ available NOTIFY stateChanged)
    Q_PROPERTY(QString error READ error NOTIFY stateChanged)
    // The update banner shows
    Q_PROPERTY(bool announce READ announce NOTIFY stateChanged)
public:
    // A version as the tags spell them: "v1.2.0", "1.1.6-beta3"
    struct Version {
        // major, minor, patch, then the beta's number (0: a final release)
        std::array<int, 4> parts{};
        bool valid = false;
        static Version parse(QString text);
        QString text() const;
        bool isBeta() const { return parts[3] != 0; }
        // The order of releases: a final comes after every beta of its number
        std::array<int, 4> rank() const { return { parts[0], parts[1], parts[2], isBeta() ? parts[3] : INT_MAX }; }
        bool operator<(const Version &later) const { return rank() < later.rank(); }
        bool operator==(const Version &same) const { return valid == same.valid && parts == same.parts; }
    };
    struct Release {
        QString tag, name, notes, page;
        Version version;
        QStringList assets;
    };

    AppUpdater(const AppContext &ctx, QObject *parent);
    ~AppUpdater() override;
    static AppUpdater *create(QQmlEngine *, QJSEngine *);

    // The releases of a feed's reply worth looking at: no drafts, https pages,
    // parsable tags. Anything but a list gives none.
    static QVector<Release> parseFeed(const QByteArray &json);
    // Whether `r` carries this app's package for `platform` ("linux", "windows";
    // -- WP QTMAC -- "macos" on macOS builds)
    static bool carriesApp(const Release &r, const QString &platform);
    // The newest release above `current` (finals only for a final), or null
    static const Release *newest(const QVector<Release> &releases, const Version &current,
                                 const QString &platform = QString());
    // The feed to use: the override if it is on this machine, else the
    // built-in one if it is https; empty when neither is allowed
    static QUrl resolveFeed(const QUrl &override, const QUrl &builtIn);
    static bool feedAllowed(const QUrl &url, bool builtIn);

    bool hasFeed() const { return m_feed.isValid() && !m_feed.isEmpty(); }
    QString feedHost() const { return m_feed.host(); }
    QByteArray userAgent() const;
    QString userAgentText() const { return QString::fromLatin1(userAgent()); }
    bool automatic() const { return m_automatic; }
    void setAutomatic(bool on);
    bool checking() const { return !m_reply.isNull(); }
    QDateTime lastChecked() const { return m_lastChecked; }
    QString currentVersion() const { return m_current.text(); }
    QString result() const { return m_result; }
    bool available() const { return m_result == QLatin1String("available"); }
    QString releaseName() const { return m_release.name; }
    QString releaseVersion() const { return available() ? m_release.version.text() : QString(); }
    QString releaseNotes() const { return m_release.notes; }
    QString releaseUrl() const { return m_release.page; }
    QString error() const { return m_error; }
    bool announce() const { return m_announce; }

    Q_INVOKABLE void checkNow();
    // The release page in the browser
    Q_INVOKABLE void openRelease();
    // Automatic checks stay quiet about the release on offer (updates/skipped)
    Q_INVOKABLE void skipVersion();
    // The banner's buttons: hide it; Details also opens Settings > Updates
    Q_INVOKABLE void later();
    Q_INVOKABLE void details();

    // For the smoke driver: another feed (refused unless allowed) and version
    void setFeed(const QUrl &url);
    void setCurrentVersionForTest(const QString &version);

signals:
    void feedChanged();
    void automaticChanged();
    void stateChanged();

private:
    void check(bool automatic);
    void launchCheck() { check(true); }
    void replied();
    // Records what a check found and tells the page and the banner
    void settle(const QString &result, const QString &why);
    void attach();

    AppContext m_ctx;
    QNetworkAccessManager *m_net;
    QTimer *m_daily;
    QPointer<QNetworkReply> m_reply;
    bool m_replyAutomatic = false;
    QUrl m_feed;
    Version m_current;
    bool m_automatic = false;
    QDateTime m_lastChecked;
    QString m_result;
    QString m_error;
    Release m_release;
    bool m_announce = false;
};

#endif // APPUPDATER_H
