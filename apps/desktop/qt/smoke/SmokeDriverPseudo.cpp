// WP QD23: the pseudo-locale run (run.sh). DSPI_SMOKE_PSEUDO=pseudo installs a
// translator, before main.qml loads, that turns every qsTr string into an
// accented, bracketed copy about a third longer ("Save" -> "[Šàṽé~~]"). The pass
// then opens the main windows and reads every Text on screen: one with plain
// ASCII words left is a string that never went through qsTr (or text that comes
// from the core or the device, which is taken out first: the core localises its
// own wording by key), and one that does not fit its box, or is cut off by a
// clipping parent or the window, is a layout that a longer language breaks.
// DSPI_SMOKE_PSEUDO=en runs the same pass untranslated, for a baseline of what
// is already cut off in English (DSPI_SMOKE_PSEUDO_BASE names its file).
#include "SmokeDriver.h"

#include "AutoEqLibrary.h"
#include "ConfigFiles.h"
#include "CsController.h"
#include "DSPiBridge.h"
#include "FirmwareUpdater.h"
#include "MonitorModel.h"
#include "Registrar.h"
#include "RtaController.h"
#include "SiggenController.h"
#include "StatsController.h"

#include <QAbstractItemModel>
#include <QCoreApplication>
#include <QMetaMethod>
#include <QFile>
#include <QMetaProperty>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QSet>
#include <QTranslator>
#include <algorithm>
#include <cmath>
#include <cstdio>

namespace {

// a-z and A-Z, each to an accented letter of the same shape
const char16_t kLower[] = u"àƀçđéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýž";
const char16_t kUpper[] = u"ÀƁÇĐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ";

// Placeholders (%1, %L1, %n), markup and entities pass through untouched
QString pseudo(const QString &source) {
    QString body;
    int letters = 0;
    for (int i = 0; i < source.size(); ++i) {
        const QChar c = source.at(i);
        if (c == QLatin1Char('%') && i + 1 < source.size()) {
            int j = i + 1;
            if (source.at(j) == QLatin1Char('L')) ++j;
            if (j < source.size() && (source.at(j).isDigit() || source.at(j) == QLatin1Char('n'))) {
                while (j + 1 < source.size() && source.at(j + 1).isDigit()) ++j;
                body += source.mid(i, j - i + 1);
                i = j;
                continue;
            }
        }
        if (c == QLatin1Char('<') || c == QLatin1Char('&')) {
            const int end = source.indexOf(c == QLatin1Char('<') ? QLatin1Char('>') : QLatin1Char(';'), i);
            if (end > i && (c == QLatin1Char('<') || end - i < 10)) {
                body += source.mid(i, end - i + 1);
                i = end;
                continue;
            }
        }
        const ushort u = c.unicode();
        if (u >= 'a' && u <= 'z') { body += QChar(kLower[u - 'a']); ++letters; }
        else if (u >= 'A' && u <= 'Z') { body += QChar(kUpper[u - 'A']); ++letters; }
        else body += c;
    }
    if (letters == 0) return source;   // "%1", "·": nothing a translator changes
    // Spaces at either end stay outside the brackets, where they were
    int lead = 0, trail = 0;
    while (lead < body.size() && body.at(lead) == QLatin1Char(' ')) ++lead;
    while (trail < body.size() - lead && body.at(body.size() - 1 - trail) == QLatin1Char(' ')) ++trail;
    const QString core = body.mid(lead, body.size() - lead - trail);
    // A sentence's padding breaks as words do, so it wraps where the text does
    const int pad = int(std::ceil(letters * 0.3));
    QString padding;
    for (int i = 0; i < pad; ++i) {
        if (i > 0 && i % 4 == 0 && core.contains(QLatin1Char(' '))) padding += QLatin1Char(' ');
        padding += QLatin1Char('~');
    }
    return QString(lead, QLatin1Char(' ')) + QLatin1Char('[') + core + padding + QLatin1Char(']')
         + QString(trail, QLatin1Char(' '));
}

class PseudoTranslator : public QTranslator {
public:
    using QTranslator::QTranslator;
    bool isEmpty() const override { return false; }
    QString translate(const char *, const char *sourceText, const char *, int) const override {
        if (!sourceText || !*sourceText) return QString();
        return pseudo(QString::fromUtf8(sourceText));
    }
};

// Every string an object's properties hold, maps and lists opened up
void collectStrings(const QVariant &v, QSet<QString> *out, int depth = 0) {
    if (depth > 6) return;
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    const int type = v.typeId();
#else
    const int type = v.userType();
#endif
    if (type == QMetaType::QString) { out->insert(v.toString()); return; }
    if (type == QMetaType::QStringList) { for (const QString &s : v.toStringList()) out->insert(s); return; }
    if (type == QMetaType::QVariantList) { for (const QVariant &x : v.toList()) collectStrings(x, out, depth + 1); return; }
    if (type == QMetaType::QVariantMap) { for (const QVariant &x : v.toMap()) collectStrings(x, out, depth + 1); return; }
    // A list model's rows (the AutoEQ results): every role of the first rows
    if (type == QMetaType::QObjectStar) {
        const auto *model = qobject_cast<const QAbstractItemModel *>(v.value<QObject *>());
        if (!model) return;
        const QList<int> roles = model->roleNames().keys();
        for (int r = 0; r < std::min(model->rowCount(), 500); ++r)
            for (int role : roles) collectStrings(model->data(model->index(r, 0), role), out, depth + 1);
    }
}

// The control surfaces' rows, as the core describes them: every record the
// model holds, under each kind the pages ask about
void collectDescriptions(const CsController *cs, const QVariant &v, int index, QSet<QString> *out, int depth = 0) {
    if (depth > 4) return;
    if (v.canConvert<QVariantList>() && v.toList().size() > 0 && v.toList().first().canConvert<QVariantMap>()) {
        const QVariantList list = v.toList();
        for (int i = 0; i < list.size(); ++i) collectDescriptions(cs, list.at(i), i, out, depth + 1);
        return;
    }
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    if (v.typeId() != QMetaType::QVariantMap) return;
#else
    if (v.userType() != QMetaType::QVariantMap) return;
#endif
    const QVariantMap rec = v.toMap();
    for (const char *kind : { "binding", "ir", "page", "step", "aux", "group", "macro", "display" })
        collectStrings(cs->describe(QString::fromLatin1(kind), index, rec), out);
    for (const QVariant &x : rec) collectDescriptions(cs, x, 0, out, depth + 1);
}

void collectObject(const QObject *o, QSet<QString> *out) {
    if (!o) return;
    const QMetaObject *mo = o->metaObject();
    for (int i = QObject::staticMetaObject.propertyCount(); i < mo->propertyCount(); ++i)
        collectStrings(mo->property(i).read(o), out);
}

QString shortClass(const QObject *o) {
    QString n = QString::fromLatin1(o->metaObject()->className());
    const int cut = n.indexOf(QLatin1String("_QML"));
    if (cut > 0) n.truncate(cut);
    return n;
}

// Where an item sits, the same from one run to the next on the demo device
QString pathOf(QQuickItem *it, QQuickItem *top) {
    QStringList parts;
    for (QQuickItem *x = it; x && x != top; x = x->parentItem()) {
        QString seg = x->objectName();
        if (seg.isEmpty()) {
            const QQuickItem *p = x->parentItem();
            seg = shortClass(x) + QLatin1Char('#') + QString::number(p ? p->childItems().indexOf(x) : 0);
        }
        parts.prepend(seg);
    }
    return parts.join(QLatin1Char('/'));
}

bool shown(QQuickItem *it) {
    if (!it->isVisible()) return false;
    double opacity = 1;
    for (QQuickItem *x = it; x; x = x->parentItem()) opacity *= x->opacity();
    return opacity > 0.05;
}

void collectTexts(QQuickItem *it, QList<QQuickItem *> *out) {
    if (!it->isVisible()) return;
    if (it->inherits("QQuickText")) out->append(it);
    for (QQuickItem *c : it->childItems()) collectTexts(c, out);
}

} // namespace

void SmokeDriver::installPseudoLocale(QCoreApplication *app) {
    if (qEnvironmentVariable("DSPI_SMOKE_PSEUDO") == QLatin1String("pseudo"))
        app->installTranslator(new PseudoTranslator(app));
}

void SmokeDriver::pseudoScan(QQuickWindow *w, QQuickItem *scope, const QString &where) {
    spin(150);
    if (!scope) scope = w->contentItem();
    const bool translated = qEnvironmentVariable("DSPI_SMOKE_PSEUDO") == QLatin1String("pseudo");

    // The core's and the device's text, as the controllers hold it now
    QSet<QString> data;
    for (const QObject *o : std::initializer_list<const QObject *>{ m_.bridge, m_.rta, m_.siggen, m_.cs, m_.firmware,
                                                                   m_.autoeq, m_.stats, m_.monitor, m_.config })
        collectObject(o, &data);
    for (const Registrar::ControllerEntry &c : Registrar::controllers())
        collectObject(m_.engine->rootContext()->contextProperty(QString::fromLatin1(c.name)).value<QObject *>(), &data);
    for (int ch = 0; ch < 32; ++ch) {
        data.insert(m_.bridge->channelName(ch));
        data.insert(m_.bridge->channelDescriptor(ch));
    }
    for (int slot = 0; slot < 16; ++slot) data.insert(m_.bridge->presetName(slot));
    // The core's onboarding and release-note words (CG10), as the views ask for them
    if (QObject *words = m_.engine->rootContext()->contextProperty(QStringLiteral("onboardingText")).value<QObject *>()) {
        for (const char *method : { "gettingStarted", "releaseNotes" }) {
            QVariant got;
            if (words->metaObject()->indexOfMethod(QByteArray(method) + "()") < 0) continue;
            const QByteArray type = words->metaObject()->method(words->metaObject()->indexOfMethod(QByteArray(method) + "()")).typeName();
            if (type == "QVariantMap") {
                QVariantMap m;
                QMetaObject::invokeMethod(words, method, Q_RETURN_ARG(QVariantMap, m));
                got = m;
            } else {
                QVariantList l;
                QMetaObject::invokeMethod(words, method, Q_RETURN_ARG(QVariantList, l));
                got = l;
            }
            collectStrings(got, &data);
        }
    }
    collectDescriptions(m_.cs, m_.cs->model(), 0, &data);
    QStringList known;
    static const QRegularExpression letter(QStringLiteral("[A-Za-z]"));
    for (const QString &s : data)
        if (s.trimmed().size() >= 2 && s.contains(letter)) known.append(s.trimmed());
    std::sort(known.begin(), known.end(), [](const QString &a, const QString &b) { return a.size() > b.size(); });
    // The core's templates whose {name}s the views fill in ("Step {current} of {total}")
    QList<QRegularExpression> templates;
    static const QRegularExpression slot(QStringLiteral("\\\\\\{\\w+\\\\\\}"));
    for (const QString &k : known)
        if (k.contains(QLatin1Char('{')))
            templates.append(QRegularExpression(QRegularExpression::escape(k).replace(slot, QStringLiteral("\\S+"))));

    static const QRegularExpression tags(QStringLiteral("<[^>]*>|&[a-z#0-9]+;"));
    static const QRegularExpression word(QStringLiteral("[A-Za-z]{2,}"));
    // Words that are the same in every language, taken out with their reason
    static const QRegularExpression same(QStringLiteral(
        "\\b(?:"
        "DSPi"                                  // the product
        "|Linux|Windows|macOS"                  // the systems, by name (About's version line)
        "|(?=[A-Za-z0-9]*[0-9])(?=[A-Za-z0-9]*[A-Za-z])[A-Za-z0-9]+"   // part and model codes: 12AX7, EL34, SSD1306, DSPI0001
        ")\\b"));

    QList<QQuickItem *> texts;
    collectTexts(scope, &texts);
    const QRectF windowRect(0, 0, w->width(), w->height());
    int count = 0, untranslated = 0, clipped = 0, inEnglish = 0, elided = 0;
    for (QQuickItem *t : texts) {
        if (!shown(t)) continue;
        QString text = t->property("text").toString();
        text.replace(tags, QStringLiteral(" "));
        if (text.trimmed().isEmpty()) continue;
        const QRectF box = t->mapRectToScene(QRectF(0, 0, t->width(), t->height()));
        if (!box.intersects(windowRect)) continue;   // parked off screen
        ++count;
        const QString key = where + QLatin1Char('|') + pathOf(t, w->contentItem());
        const QString shownText = text.simplified();

        if (translated) {
            QString rest = text;
            for (const QString &k : known) rest.replace(k, QStringLiteral(" "));
            for (const QRegularExpression &t : templates) rest.replace(t, QStringLiteral(" "));
            rest.replace(same, QStringLiteral(" "));
            if (rest.contains(word)) {
                ++untranslated;
                printf("smoke: QD23 untranslated: %s: \"%s\"\n", qPrintable(key), qPrintable(shownText));
            }
        }

        // Fits its box, and the box is not cut by a clipping parent or the window
        QString why;
        const double cw = t->property("contentWidth").toDouble();
        const double chh = t->property("contentHeight").toDouble();
        if (t->property("truncated").toBool()) why = QStringLiteral("elided");
        else if (t->width() > 0 && cw > t->width() + 1.5) why = QStringLiteral("%1 px wider than its box").arg(qRound(cw - t->width()));
        else if (t->height() > 0 && chh > t->height() + 3) why = QStringLiteral("%1 px taller than its box").arg(qRound(chh - t->height()));
        else {
            const int align = t->property("horizontalAlignment").toInt();
            const double w0 = std::min(cw, t->width());
            const double x0 = align == Qt::AlignRight ? t->width() - w0 : align == Qt::AlignHCenter ? (t->width() - w0) / 2 : 0;
            const QRectF painted = t->mapRectToScene(QRectF(x0, 0, w0, std::min(chh, t->height())));
            for (QQuickItem *p = t->parentItem(); p && why.isEmpty(); p = p->parentItem()) {
                if (p->inherits("QQuickFlickable")) break;   // scrolls into view
                if (p->clip() && !p->mapRectToScene(QRectF(0, 0, p->width(), p->height())).adjusted(-1, -1, 1, 1).contains(painted))
                    why = QStringLiteral("cut off by %1").arg(p->objectName().isEmpty() ? shortClass(p) : p->objectName());
                if (!p->parentItem() && !windowRect.adjusted(-1, -1, 1, 1).contains(painted))
                    why = QStringLiteral("cut off by the window");
            }
        }
        if (why.isEmpty()) continue;
        // An ellipsis the design put there (a long name in a fixed column) loses
        // text but shows that it did: listed, not failed. Cut off or spilling is.
        if (why == QLatin1String("elided")) {
            ++elided;
            printf("smoke: QD23 elided: %s: \"%s\"\n", qPrintable(key), qPrintable(shownText));
            continue;
        }
        m_pseudoClipped.append(key);
        if (m_pseudoBaseline.contains(key)) {
            ++inEnglish;
            printf("smoke: QD23 clipped in English too: %s: %s: \"%s\"\n", qPrintable(key), qPrintable(why), qPrintable(shownText));
        } else {
            ++clipped;
            printf("smoke: QD23 clipped: %s: %s: \"%s\"\n", qPrintable(key), qPrintable(why), qPrintable(shownText));
        }
    }
    QString file = where;
    file.replace(QRegularExpression(QStringLiteral("[^A-Za-z0-9]+")), QStringLiteral("-"));
    w->grabWindow().save(m_out + QStringLiteral("/pseudo-%1.png").arg(file));
    printf("smoke: QD23 %s: %d texts, %d untranslated, %d clipped (%d also in English), %d elided\n", qPrintable(where),
           count, untranslated, clipped, inEnglish, elided);
    check(qPrintable(QStringLiteral("QD23: %1: every text translated and whole").arg(where)),
          count > 0 && untranslated == 0 && (translated ? clipped == 0 : true));
}

void SmokeDriver::runPseudoLocale() {
    section("QD23: pseudo-locale, the main windows");
    const bool translated = qEnvironmentVariable("DSPI_SMOKE_PSEUDO") == QLatin1String("pseudo");
    QFile base(qEnvironmentVariable("DSPI_SMOKE_PSEUDO_BASE"));
    if (translated && base.open(QIODevice::ReadOnly))
        for (const QByteArray &line : base.readAll().split('\n'))
            if (!line.isEmpty()) m_pseudoBaseline.insert(QString::fromUtf8(line));
    printf("smoke: QD23 %s run, %d English baseline entries\n", translated ? "pseudo-locale" : "English",
           int(m_pseudoBaseline.size()));

    auto *main = qobject_cast<QQuickWindow *>(root());
    check("QD23: the main window", main != nullptr);
    if (main) {
        const auto hideTools = [&]() {
            for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
                if (w != main && w->isVisible()) w->setVisible(false);
            spin(50);
        };
        hideTools();
        main->show();
        QMetaObject::invokeMethod(root(), "selectOverview");
        spin(600);
        pseudoScan(main, nullptr, QStringLiteral("main window"));
        QMetaObject::invokeMethod(root(), "selectChannel", Q_ARG(QVariant, 0));
        spin(600);
        pseudoScan(main, nullptr, QStringLiteral("main window, an input"));
        QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 0));
        spin(600);
        pseudoScan(main, nullptr, QStringLiteral("main window, an output"));
        QMetaObject::invokeMethod(root(), "selectOverview");
        spin(300);

        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("devices")));
        spin(400);
        QObject *devices = root()->findChild<QObject *>(QStringLiteral("devicesPopup"));
        QQuickItem *devicesItem = devices ? devices->property("contentItem").value<QQuickItem *>() : nullptr;
        if (devicesItem) pseudoScan(main, devicesItem->parentItem(), QStringLiteral("devices"));
        if (devices) QMetaObject::invokeMethod(devices, "close");
        spin(200);

        // Settings, every page this device has
        QQuickWindow *settings = nullptr;
        for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
            if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
        check("QD23: the Settings window", settings != nullptr);
        if (settings) {
            QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("settings")));
            spin(400);
            for (const QVariant &g : settings->property("groups").toList())
                for (const QVariant &p : g.toMap().value(QStringLiteral("pages")).toList()) {
                    const QString id = p.toMap().value(QStringLiteral("id")).toString();
                    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, id));
                    spin(400);
                    if (settings->property("currentPage").toMap().value(QStringLiteral("id")).toString() != id) continue;
                    pseudoScan(settings, nullptr, QStringLiteral("settings: %1").arg(id));
                }
            settings->setVisible(false);
            spin(100);
        }

        for (const char *n : { "matrix", "graph", "loudness", "crossfeed", "leveller", "psybass", "subharm", "tube", "upmix",
                               "stats", "spectrum", "siggen", "monitor", "firmware", "whatsNew", "autoeq", "autoeqRebuild" }) {
            hideTools();
            QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QString::fromLatin1(n)));
            spin(600);
            QQuickWindow *found = nullptr;
            for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
                if (w != main && w->isVisible()) found = w;
            check(qPrintable(QStringLiteral("QD23: the %1 window").arg(QString::fromLatin1(n))), found != nullptr);
            if (found) pseudoScan(found, nullptr, QString::fromLatin1(n));
        }
        hideTools();

        // The first-launch guide, over the main window
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("gettingStarted")));
        spin(500);
        pseudoScan(main, nullptr, QStringLiteral("getting started"));
        root()->setProperty("setupRequested", false);
        spin(100);
    }

    if (!translated) {
        QFile out(m_out + QStringLiteral("/clipped.txt"));
        if (out.open(QIODevice::WriteOnly | QIODevice::Truncate)) out.write(m_pseudoClipped.join(QLatin1Char('\n')).toUtf8());
    }
    sectionTimes();
    m_failures += qmlErrors();
    printf("smoke: %d checks, %d failures (%d QML errors)\n", m_checks, m_failures, qmlErrors());
    fflush(stdout);
    QCoreApplication::exit(m_failures ? 1 : 0);
}
