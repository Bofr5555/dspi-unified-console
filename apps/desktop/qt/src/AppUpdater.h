#ifndef APPUPDATER_H
#define APPUPDATER_H

#include <QByteArray>
#include <QDateTime>
#include <QObject>
#include <QString>
#include <QTimer>
#include <QUrl>
#include <QVector>

class DSPiBridge;
class QNetworkAccessManager;
class QNetworkReply;

// WP QD16: tells the user when a newer DSPi Console is published, shows its notes
// and opens its page (AppUpdateChecker.swift, without the installer: OD8 says
// check-and-link). The feed is a GitHub release list (GET /repos/../releases).
//
// The check is opt-in: nothing is fetched until the user turns on automatic
// checks or presses Check Now. privacyLine says what a check sends, and the
// request is built to send exactly that: no cookies, no cache, the User-Agent
// and Accept-Language set here rather than left to Qt.
//
// Not built when CMake has DSPI_UPDATE_CHECK=OFF (distro packages update the
// app themselves): then there is no `appUpdater` and its page and banner hide.
class AppUpdater : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool automatic READ automatic WRITE setAutomatic NOTIFY automaticChanged)
    Q_PROPERTY(bool hasFeed READ hasFeed NOTIFY feedChanged)
    Q_PROPERTY(QString feedHost READ feedHost NOTIFY feedChanged)
    Q_PROPERTY(QString privacyLine READ privacyLine NOTIFY feedChanged)
    Q_PROPERTY(QString currentVersion READ currentVersion CONSTANT)
    Q_PROPERTY(bool checking READ checking NOTIFY stateChanged)
    // After a check: "" (never checked), "current", "available" or "failed"
    Q_PROPERTY(QString result READ result NOTIFY stateChanged)
    Q_PROPERTY(QString failure READ failure NOTIFY stateChanged)
    Q_PROPERTY(QString lastChecked READ lastChecked NOTIFY stateChanged)
    // The release on offer, while result is "available"
    Q_PROPERTY(QString availableVersion READ availableVersion NOTIFY stateChanged)
    Q_PROPERTY(QString availableTitle READ availableTitle NOTIFY stateChanged)
    Q_PROPERTY(QString notes READ notes NOTIFY stateChanged)
    Q_PROPERTY(QString pageUrl READ pageUrl NOTIFY stateChanged)
    // For the banner: an update an automatic check found, not skipped, not dismissed
    Q_PROPERTY(bool announce READ announce NOTIFY stateChanged)

public:
    // A version as the tags spell it: "v1.1.7", "1.1.6-beta4". beta 0 is a final
    // release, which sorts above its own betas.
    struct Version {
        int major = 0, minor = 0, patch = 0, beta = 0;
        bool valid = false;
        static Version parse(const QString &text);
        bool operator<(const Version &o) const;
        QString text() const;
    };
    struct Release {
        QString tag, title, notes;
        QUrl page;
        Version version;
    };

    explicit AppUpdater(DSPiBridge *bridge, QObject *parent = nullptr);

    // The releases in a feed's answer. Drafts, tags that are not versions and
    // pages that are not http(s) are left out; anything but a JSON array is none.
    static QVector<Release> parseFeed(const QByteArray &json);
    // The newest release to offer over `current`. Someone on a final release is
    // offered finals only, someone on a beta newer betas too (Swift's rule).
    static const Release *newest(const QVector<Release> &releases, const Version &current);

    bool automatic() const { return m_automatic; }
    void setAutomatic(bool on);
    bool hasFeed() const { return m_feed.isValid(); }
    QString feedHost() const { return m_feed.host(); }
    QString privacyLine() const;
    QString currentVersion() const { return m_current.text(); }
    bool checking() const { return m_reply != nullptr; }
    QString result() const { return m_result; }
    QString failure() const { return m_failure; }
    QString lastChecked() const;
    QString availableVersion() const { return m_offer.version.text(); }
    QString availableTitle() const { return m_offer.title; }
    QString notes() const { return m_offer.notes; }
    QString pageUrl() const { return m_offer.page.toString(); }
    bool announce() const;

    Q_INVOKABLE void checkNow();
    Q_INVOKABLE void openPage();
    Q_INVOKABLE void skipVersion();
    Q_INVOKABLE void dismiss();

    // What the requests send as User-Agent; privacyLine names it
    QByteArray userAgent() const;
    // The feed to ask. Empty: the one built in (CMake DSPI_UPDATE_FEED_URL), which
    // the settings key updates/feedUrl overrides, for a local feed.
    void setFeed(const QUrl &feed);
    void setCurrentVersionForTest(const QString &version) { m_current = Version::parse(version); }

signals:
    void automaticChanged();
    void feedChanged();
    void stateChanged();

private:
    void check(bool automatic);
    void finished();
    void setFailure(const QString &why);

    QUrl m_feed;
    Version m_current;
    bool m_automatic = false;
    bool m_fromAutomatic = false;
    bool m_dismissed = false;
    QString m_skipped;
    QString m_result;
    QString m_failure;
    QDateTime m_lastChecked;
    Release m_offer;
    QNetworkAccessManager *m_net = nullptr;
    QNetworkReply *m_reply = nullptr;
    QTimer m_daily;
    QTimer m_launch;
    QTimer m_timeout;
};

#endif // APPUPDATER_H
