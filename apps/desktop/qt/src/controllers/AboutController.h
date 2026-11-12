#ifndef ABOUTCONTROLLER_H
#define ABOUTCONTROLLER_H

#include <QObject>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// Settings > About: the core's About table (dspi_about_json, assets/about.json:
// credit lines, makers, links, credits) and the third-party notices the
// packaging staged beside the app, each opened in a text viewer
// (behaviour.md §27). The page shows the table's wording; nothing here
// rewrites it.
//
// Where the notices are looked for, first match wins: $DSPI_NOTICES_DIR; the
// executable's folder (Windows, the AppImage: NOTICE.txt and LICENSES/ beside
// it); ../Resources (macOS); ../share/doc/uconsole-for-dspi (Linux packages); and
// notices/ beside the executable (a development build's copy, cmake/wp/qp10.cmake).
class AboutController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool draft READ draft CONSTANT)
    Q_PROPERTY(QStringList creditLines READ creditLines CONSTANT)
    Q_PROPERTY(QVariantList makers READ makers CONSTANT)
    Q_PROPERTY(QString linksTitle READ linksTitle CONSTANT)
    Q_PROPERTY(QString linksNote READ linksNote CONSTANT)
    Q_PROPERTY(QVariantList links READ links CONSTANT)
    Q_PROPERTY(QVariantList credits READ credits CONSTANT)
    // "for Linux", "for Windows" (behaviour.md §25)
    Q_PROPERTY(QString platformLabel READ platformLabel CONSTANT)
    // [{title, detail, path}]: one row per staged notice file
    Q_PROPERTY(QVariantList notices READ notices NOTIFY noticesChanged)
    Q_PROPERTY(QString noticesDir READ noticesDir NOTIFY noticesChanged)
    // The notice the viewer shows
    Q_PROPERTY(QString noticeTitle READ noticeTitle NOTIFY noticeChanged)
    Q_PROPERTY(QString noticeText READ noticeText NOTIFY noticeChanged)
    Q_PROPERTY(bool noticeLoading READ noticeLoading NOTIFY noticeChanged)
public:
    AboutController(const AppContext &ctx, QObject *parent);
    static AboutController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<AboutController>(); }

    bool draft() const { return m_draft; }
    QStringList creditLines() const { return m_creditLines; }
    QVariantList makers() const { return m_makers; }
    QString linksTitle() const { return m_linksTitle; }
    QString linksNote() const { return m_linksNote; }
    QVariantList links() const { return m_links; }
    QVariantList credits() const { return m_credits; }
    QString platformLabel() const;
    QVariantList notices() const { return m_notices; }
    QString noticesDir() const { return m_noticesDir; }
    QString noticeTitle() const { return m_noticeTitle; }
    QString noticeText() const { return m_noticeText; }
    bool noticeLoading() const { return m_noticeLoading; }

    // Reads notice `index` off the GUI thread; noticeChanged when it is in
    Q_INVOKABLE void openNotice(int index);
    // Looks for the notices again (the page's onVisible)
    Q_INVOKABLE void rescanNotices();
    // A web link of the table, in the browser (https only, as the shell's)
    Q_INVOKABLE void openLink(const QString &url);

    // The notice rows of folder `root` (tests and the smoke driver)
    static QVariantList noticesIn(const QString &root);

signals:
    void noticesChanged();
    void noticeChanged();

private:
    void readTable();

    AppContext m_ctx;
    bool m_draft = false;
    QStringList m_creditLines;
    QVariantList m_makers;
    QString m_linksTitle;
    QString m_linksNote;
    QVariantList m_links;
    QVariantList m_credits;
    QVariantList m_notices;
    QString m_noticesDir;
    QString m_noticeTitle;
    QString m_noticeText;
    bool m_noticeLoading = false;
    int m_noticeRequest = 0;
};

#endif // ABOUTCONTROLLER_H
