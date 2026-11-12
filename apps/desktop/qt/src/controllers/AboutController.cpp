// -- WP QP10 -- Settings > About: the core's About table and the notices list
#include "AboutController.h"

#include "AppIdentity.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QCoreApplication>
#include <QDir>
#include <QDirIterator>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>
#include <QThreadPool>

namespace {

const Registrar::Controller<AboutController> aboutController;

// A notice larger than this is shown cut, with a line saying so
constexpr qint64 kMaxNoticeBytes = 8 * 1024 * 1024;

QString text(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }

// The folder holding NOTICE.txt and LICENSES/, or "" when none is found
QString findNotices() {
    QStringList roots;
    const QString env = qEnvironmentVariable("DSPI_NOTICES_DIR");
    if (!env.isEmpty()) roots << env;
    const QString app = QCoreApplication::applicationDirPath();
    roots << app << app + QStringLiteral("/../Resources") << app + QStringLiteral("/../share/doc/dspi-uconsole")
          << app + QStringLiteral("/notices");
    for (const QString &r : std::as_const(roots)) {
        const QDir dir(r);
        if (dir.exists(QStringLiteral("LICENSES")) || dir.exists(QStringLiteral("NOTICE.txt")))
            return dir.canonicalPath();
    }
    return {};
}

} // namespace

AboutController::AboutController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    readTable();
    rescanNotices();
}

void AboutController::readTable() {
    // The table is compiled into the core; no device, no lock
    const QJsonObject t =
        QJsonDocument::fromJson(readSized([](char *buf, uint32_t len) { return dspi_about_json(buf, len); }, 8192))
            .object();
    m_draft = t.value(QLatin1String("draft")).toBool();
    for (const QJsonValue &v : t.value(QLatin1String("credit_lines")).toArray()) m_creditLines << text(v.toObject(), "text");
    for (const QJsonValue &v : t.value(QLatin1String("makers")).toArray()) {
        const QJsonObject m = v.toObject();
        QStringList made;
        for (const QJsonValue &w : m.value(QLatin1String("made")).toArray()) made << w.toString();
        m_makers.append(QVariantMap{ { QStringLiteral("name"), text(m, "name") },
                                     { QStringLiteral("url"), text(m, "url") },
                                     { QStringLiteral("made"), made } });
    }
    m_linksTitle = text(t, "links_title");
    m_linksNote = text(t, "links_note");
    for (const QJsonValue &v : t.value(QLatin1String("links")).toArray()) {
        const QJsonObject l = v.toObject();
        m_links.append(QVariantMap{ { QStringLiteral("id"), text(l, "id") },
                                    { QStringLiteral("title"), text(l, "title") },
                                    { QStringLiteral("detail"), text(l, "detail") },
                                    { QStringLiteral("url"), text(l, "url") } });
    }
    for (const QJsonValue &v : t.value(QLatin1String("credits")).toArray()) {
        const QJsonObject c = v.toObject();
        m_credits.append(QVariantMap{ { QStringLiteral("name"), text(c, "name") },
                                      { QStringLiteral("what"), text(c, "what") },
                                      { QStringLiteral("url"), text(c, "url") } });
    }
}

QString AboutController::platformLabel() const {
#if defined(Q_OS_WIN)
    return tr("for Windows");
#elif defined(Q_OS_MACOS)
    return tr("for macOS");
#else
    return tr("for Linux");
#endif
}

QVariantList AboutController::noticesIn(const QString &root) {
    QVariantList rows;
    if (root.isEmpty()) return rows;
    const QDir dir(root);
    // The names the packages stage (packaging/licences/stage.sh), with the
    // Mac's titles where it has one; any other file is listed by its name and
    // the component folder it sits in
    struct Known {
        const char *path;
        const char *title;
        const char *detail;
    };
    static const Known known[] = {
        { "NOTICE.txt", QT_TRANSLATE_NOOP("AboutController", "Notice"), nullptr },
        { "LICENSES/GPL-3.0.txt", QT_TRANSLATE_NOOP("AboutController", "GNU General Public License, version 3"), nullptr },
        { "LICENSES/TERMS.txt", QT_TRANSLATE_NOOP("AboutController", "Additional terms"), nullptr },
        { "LICENSES/LGPL-3.0.txt", QT_TRANSLATE_NOOP("AboutController", "GNU Lesser General Public License, version 3"), "Qt" },
        { "LICENSES/rust-crates.txt", QT_TRANSLATE_NOOP("AboutController", "Rust crates: licences and notices"),
          QT_TRANSLATE_NOOP("AboutController", "The crates the core links") },
        { "LICENSES/BUNDLED.txt", QT_TRANSLATE_NOOP("AboutController", "Bundled libraries"), nullptr },
    };
    QStringList listed;
    for (const Known &k : known) {
        const QString path = QString::fromLatin1(k.path);
        if (!QFileInfo(dir.filePath(path)).isFile()) continue;
        listed << path;
        rows.append(QVariantMap{
            { QStringLiteral("title"), QCoreApplication::translate("AboutController", k.title) },
            { QStringLiteral("detail"),
              k.detail ? QCoreApplication::translate("AboutController", k.detail) : AppIdentity::displayName() },
            { QStringLiteral("path"), dir.filePath(path) } });
    }
    QStringList others;
    QDirIterator it(dir.filePath(QStringLiteral("LICENSES")), QDir::Files, QDirIterator::Subdirectories);
    while (it.hasNext()) {
        const QString rel = dir.relativeFilePath(it.next());
        if (!listed.contains(rel)) others << rel;
    }
    others.sort(Qt::CaseInsensitive);
    for (const QString &rel : std::as_const(others)) {
        const QFileInfo info(rel);
        // LICENSES/<component>/.../<file>, or a file straight in LICENSES/
        const QStringList parts = rel.split(QLatin1Char('/'));
        rows.append(QVariantMap{ { QStringLiteral("title"), info.fileName() },
                                 { QStringLiteral("detail"), parts.size() > 2 ? parts.at(1) : QString() },
                                 { QStringLiteral("path"), dir.filePath(rel) } });
    }
    return rows;
}

void AboutController::rescanNotices() {
    const QString root = findNotices();
    QVariantList rows = noticesIn(root);
    if (root == m_noticesDir && rows == m_notices) return;
    m_noticesDir = root;
    m_notices = rows;
    emit noticesChanged();
}

void AboutController::openNotice(int index) {
    if (index < 0 || index >= m_notices.size()) return;
    const QVariantMap row = m_notices.at(index).toMap();
    const QString path = row.value(QStringLiteral("path")).toString();
    m_noticeTitle = row.value(QStringLiteral("title")).toString();
    m_noticeText.clear();
    m_noticeLoading = true;
    const int request = ++m_noticeRequest;
    emit noticeChanged();

    // Files stay off the GUI thread
    QPointer<AboutController> self(this);
    QThreadPool::globalInstance()->start([self, path, request]() {
        QFile file(path);
        QString body;
        if (file.open(QIODevice::ReadOnly)) {
            body = QString::fromUtf8(file.read(kMaxNoticeBytes));
            if (!file.atEnd()) body += QLatin1Char('\n') + AboutController::tr("(The rest of this file is not shown.)");
        } else {
            body = AboutController::tr("This notice could not be read: %1").arg(file.errorString());
        }
        QMetaObject::invokeMethod(
            QCoreApplication::instance(),
            [self, body, request]() {
                if (!self || request != self->m_noticeRequest) return;
                self->m_noticeText = body;
                self->m_noticeLoading = false;
                emit self->noticeChanged();
            },
            Qt::QueuedConnection);
    });
}

void AboutController::openLink(const QString &url) { m_ctx.shell->openUrl(url); }
