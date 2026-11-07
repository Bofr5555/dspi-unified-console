// WP QD33: the About page shows what the core's about table holds, as written, and
// holds up under wording nobody has written yet. Links go to a handler here: this
// pass opens nothing.
#include "SmokeDriver.h"

#include "AboutController.h"

#include <QDesktopServices>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSet>
#include <QUrl>
#include <cstdio>
#include <functional>

extern "C" {
#include "dspi_core.h"
}

namespace {
class UrlCatcher : public QObject {
    Q_OBJECT
public:
    QStringList urls;
public slots:
    void open(const QUrl &url) { urls.append(url.toString()); }
};

void walk(QQuickItem *from, const std::function<void(QQuickItem *)> &visit) {
    visit(from);
    for (QQuickItem *c : from->childItems()) walk(c, visit);
}

QQuickItem *visiblePage(QQuickItem *from) {
    if (from->isVisible() && from->inherits("QQuickFlickable") && from->property("writesToDevice").isValid()) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = visiblePage(c)) return found;
    return nullptr;
}

// Everything the page shows as text
QStringList shownTexts(QQuickItem *page) {
    QStringList out;
    walk(page, [&](QQuickItem *i) {
        if (i->inherits("QQuickText") && i->isVisible()) out.append(i->property("text").toString());
    });
    return out;
}

QList<QQuickItem *> rowsOf(QQuickItem *page) {
    QList<QQuickItem *> out;
    walk(page, [&](QQuickItem *i) {
        if (i->isVisible() && i->property("clickable").isValid() && i->property("verbatim").isValid()) out.append(i);
    });
    return out;
}

// No text past the page's right edge or wider than its own box, and the page
// scrolls to the end of everything
bool fits(QQuickItem *page) {
    bool ok = true;
    walk(page, [&](QQuickItem *i) {
        if (!i->inherits("QQuickText") || !i->isVisible() || i->property("text").toString().isEmpty()) return;
        const QPointF end = i->mapToItem(page, QPointF(i->width(), i->height()));
        const bool inside = end.x() <= page->width() + 0.5
            && i->property("contentWidth").toReal() <= i->width() + 1.0;
        const qreal bottom = end.y() + page->property("contentY").toReal();
        if (!inside || bottom > page->property("contentHeight").toReal() + 0.5) {
            printf("  about: does not fit: \"%s\"\n", qPrintable(i->property("text").toString().left(40)));
            ok = false;
        }
    });
    return ok;
}

// Wording nothing like the draft's: very long, without spaces, looks like markup,
// fields missing, keys nobody knows, a whole list gone
const char kOddWording[] = R"({
  "future_field": 7,
  "credit_lines": [
    { "text": "<b>Not bold</b> &amp; <a href=\"https://example.invalid/\">not a link</a>" },
    { "text": "A credit line long enough to need several lines on the card, and then some more so that it certainly does, whatever the width of the window, because it goes on and on and on and on and on and on and on and on and on and on" },
    { "text": "" },
    { "id": "no_text" }
  ],
  "makers": [
    { "name": "Unbroken", "made": ["Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] },
    { "name": "<i>Maker</i>", "url": "https://example.invalid/maker", "made": ["One", "Two, with a comma"] },
    { "made": "not a list", "name": 5 }
  ],
  "credits": [
    { "name": "No link", "what": "<b>literally</b>" },
    { "name": "Linked", "what": "x", "url": "https://example.invalid/credit" }
  ]
})";
}

void SmokeDriver::testAbout() {
    section("about: the core's table, as written (QD33)");
    auto *about = qobject_cast<AboutController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("aboutInfo")).value<QObject *>());
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    check("QD33: the controller and the settings window", about && settings);
    if (!about || !settings) return;

    // The table, read here on its own, not through the controller
    QByteArray raw(int(dspi_about_json(nullptr, 0)), '\0');
    check("QD33: the core has a table", !raw.isEmpty()
          && dspi_about_json(raw.data(), uint32_t(raw.size())) == uint32_t(raw.size()));
    raw.chop(1);
    const QJsonObject table = QJsonDocument::fromJson(raw).object();
    const auto list = [&table](const char *key) { return table.value(QLatin1String(key)).toArray(); };
    const auto str = [](const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); };

    settings->setVisible(true);
    spin(200);
    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("advanced")));
    spin(100);
    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("about")));
    spin(300);
    QQuickItem *page = visiblePage(settings->contentItem());
    check("QD33: the page opens", page != nullptr);
    if (!page) { settings->setVisible(false); return; }
    check("QD33: live on read-only firmware too", !page->property("writesToDevice").toBool());

    // Every string the table holds is on the page, whole and unchanged
    QStringList wanted, urls;
    int rowCount = 0;
    for (const QJsonValue &v : list("credit_lines")) wanted << str(v.toObject(), "text");
    for (const QJsonValue &v : list("makers")) {
        const QJsonObject o = v.toObject();
        QStringList made;
        const QJsonArray items = o.value(QLatin1String("made")).toArray();
        for (const QJsonValue &m : items) made << m.toString();
        wanted << str(o, "name") << made.join(QLatin1Char('\n'));
        urls << str(o, "url");
        rowCount++;
    }
    for (const QJsonValue &v : list("links")) {
        const QJsonObject o = v.toObject();
        wanted << str(o, "title") << str(o, "detail");
        urls << str(o, "url");
        rowCount++;
    }
    for (const QJsonValue &v : list("credits")) {
        const QJsonObject o = v.toObject();
        wanted << str(o, "name") << str(o, "what");
        if (o.contains(QLatin1String("url"))) urls << str(o, "url");
        rowCount++;
    }
    const QStringList shown = shownTexts(page);
    int missing = 0;
    for (const QString &w : wanted)
        if (!shown.contains(w)) { printf("  about: not on the page: \"%s\"\n", qPrintable(w)); missing++; }
    check("QD33: every string in the table is on the page", !wanted.isEmpty() && missing == 0);
    check("QD33: a row for each maker, link and credit", rowsOf(page).size() == rowCount);

    // Each row with a link opens that link, and only those are clickable
    UrlCatcher catcher;
    QDesktopServices::setUrlHandler(QStringLiteral("https"), &catcher, "open");
    const auto activateAll = [&]() {
        catcher.urls.clear();
        int clickable = 0;
        bool iconsRight = true;
        for (QQuickItem *row : rowsOf(page)) {
            const bool can = row->property("clickable").toBool();
            iconsRight = iconsRight && can == (row->property("trailingWidth").toReal() > 0);
            if (can) {
                clickable++;
                QMetaObject::invokeMethod(row, "activated");
            }
        }
        check("QD33: a link icon on exactly the rows with a link", iconsRight);
        return clickable;
    };
    check("QD33: the rows with a URL are the clickable ones", activateAll() == urls.size());
    check("QD33: they open the table's URLs", catcher.urls.size() == urls.size()
          && QSet<QString>(catcher.urls.begin(), catcher.urls.end()) == QSet<QString>(urls.begin(), urls.end()));

    // The page holds no copy of what moved to the table. A very short string, such
    // as the credit "Qt", says nothing: QML is full of Qt.rgba.
    QFile source(QStringLiteral(":/qml/settings/pages/AboutPage.qml"));
    const QByteArray qml = source.open(QIODevice::ReadOnly) ? source.readAll() : QByteArray();
    check("QD33: the page's source is readable", !qml.isEmpty());
    bool copy = false;
    for (const QStringList &strings : { wanted, urls })
        for (const QString &s : strings)
            if (s.size() >= 4 && qml.contains(s.toUtf8())) {
                printf("  about: in the page's source: \"%s\"\n", qPrintable(s));
                copy = true;
            }
    check("QD33: no string of the table in the page's source", !copy);
    check("QD33: the draft fits", fits(page));
    settings->grabWindow().save(m_out + QStringLiteral("/about-draft.png"));

    // Other wording: shown literally, all of it fits, fields left out do no harm
    const int errorsBefore = qmlErrors();
    about->load(kOddWording);
    spin(300);
    const QStringList odd = shownTexts(page);
    check("QD33: markup stays text", odd.contains(QStringLiteral("<b>Not bold</b> &amp; <a href=\"https://example.invalid/\">not a link</a>"))
          && odd.contains(QStringLiteral("<i>Maker</i>")) && odd.contains(QStringLiteral("<b>literally</b>")));
    check("QD33: made things read one to a line", odd.contains(QStringLiteral("One\nTwo, with a comma")));
    check("QD33: an empty list hides its section", !odd.contains(QStringLiteral("Links")) && odd.contains(QStringLiteral("Credits")));
    QQuickItem *card = nullptr;
    walk(page, [&](QQuickItem *i) { if (i->objectName() == QLatin1String("aboutIdentity")) card = i; });
    check("QD33: the card grows with its lines", card && card->height() > 116);
    bool cardHolds = card != nullptr;
    if (card) {
        walk(card, [&](QQuickItem *i) {
            if (i != card && i->isVisible() && i->inherits("QQuickText")) {
                const QPointF top = i->mapToItem(card, QPointF(0, 0));
                cardHolds = cardHolds && top.y() >= 0 && top.y() + i->height() <= card->height();
            }
        });
    }
    check("QD33: the card holds its lines", cardHolds);
    check("QD33: other wording fits", fits(page));
    const int oddClickable = activateAll();
    check("QD33: only rows with a URL are clickable", oddClickable == 2
          && catcher.urls == QStringList({ QStringLiteral("https://example.invalid/maker"), QStringLiteral("https://example.invalid/credit") }));
    settings->grabWindow().save(m_out + QStringLiteral("/about-odd-wording.png"));

    // Nothing usable at all: the app's own name stays, no section does
    about->load("not json");
    spin(200);
    check("QD33: an empty table leaves no row", rowsOf(page).isEmpty() && about->makers().isEmpty());
    check("QD33: the name stays", shownTexts(page).contains(QStringLiteral("DSPi Console")));
    check("QD33: no QML error from any of it", qmlErrors() == errorsBefore);

    QDesktopServices::unsetUrlHandler(QStringLiteral("https"));
    about->load(raw);
    spin(100);
    check("QD33: back to the table", rowsOf(page).size() == rowCount);
    settings->setVisible(false);
}

#include "SmokeDriverAbout.moc"
