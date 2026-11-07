#include "AboutController.h"
#include "Registrar.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QVariantMap>
#include <QtDebug>

extern "C" {
#include "dspi_core.h"
}

static const Registrar::Controller<AboutController> aboutRegistered("aboutInfo");

namespace {
QByteArray coreAbout() {
    QByteArray buf(8 * 1024, '\0');
    uint32_t needed = dspi_about_json(buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) {
        buf.resize(int(needed));
        needed = dspi_about_json(buf.data(), uint32_t(buf.size()));
    }
    if (needed == 0 || needed > uint32_t(buf.size())) return {};
    buf.truncate(int(needed - 1));
    return buf;
}

QString field(const QJsonObject &o, const char *key) {
    return o.value(QLatin1String(key)).toString();
}

void addRow(QVariantList *rows, const QString &title, const QString &detail, const QString &url) {
    if (title.isEmpty() && detail.isEmpty()) return;
    rows->append(QVariantMap{ { QStringLiteral("title"), title },
                              { QStringLiteral("detail"), detail },
                              { QStringLiteral("url"), url } });
}
}

AboutController::AboutController(DSPiBridge *, QObject *parent)
    : QObject(parent)
{
    load(coreAbout());
}

void AboutController::load(const QByteArray &json) {
    QJsonParseError error{};
    const QJsonDocument doc = QJsonDocument::fromJson(json, &error);
    if (!doc.isObject()) qWarning("About: the table did not parse: %s", qPrintable(error.errorString()));
    const QJsonObject about = doc.object();

    m_creditLines.clear();
    m_makers.clear();
    m_links.clear();
    m_credits.clear();

    const auto list = [&about](const char *key) { return about.value(QLatin1String(key)).toArray(); };
    for (const QJsonValue &v : list("credit_lines")) {
        const QString text = field(v.toObject(), "text");
        if (!text.isEmpty()) m_creditLines.append(text);
    }
    for (const QJsonValue &v : list("makers")) {
        const QJsonObject o = v.toObject();
        // One to a line: joined with a comma they would no longer read as the table has them
        QStringList made;
        const QJsonArray items = o.value(QLatin1String("made")).toArray();
        for (const QJsonValue &item : items) made.append(item.toString());
        addRow(&m_makers, field(o, "name"), made.join(QLatin1Char('\n')), field(o, "url"));
    }
    for (const QJsonValue &v : list("links")) {
        const QJsonObject o = v.toObject();
        addRow(&m_links, field(o, "title"), field(o, "detail"), field(o, "url"));
    }
    for (const QJsonValue &v : list("credits")) {
        const QJsonObject o = v.toObject();
        addRow(&m_credits, field(o, "name"), field(o, "what"), field(o, "url"));
    }
    emit changed();
}
