// -- WP QP18 -- The accessibility, localisation and light-scheme passes over
// every window: the main window (overview, an input and an output page), every
// registered tool window and every Settings page.
// - Every control that takes the keyboard has a name for assistive
//   technology, and Tab walks through each window and out of every control
//   (no keyboard trap, WCAG 2.1.2).
// - A screenshot of each in the light scheme (light/).
// - The pseudo-locale pass (qp18-pseudo, behaviour.md §22): a translator
//   that marks every string the app translates; each window again (pseudo/),
//   with the texts that came through unmarked and the ones cut short.
// -- WP LAY7 -- Then every tab of Modern's sections in the main window.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "PaletteController.h"   // -- WP LAY7 --
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QAccessible>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QtQml/QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <QTranslator>
#include <functional>

namespace {

// Every string through it comes out as [Ŝţŕîñĝ~~~]: accented, bracketed and a
// third longer. Placeholders (%1, %n, %L1) and the mnemonic '&' stay as they are.
class PseudoTranslator : public QTranslator {
public:
    bool isEmpty() const override { return false; }
    QString translate(const char *, const char *source, const char *, int) const override {
        if (!source || !*source) return QString();
        return pseudo(QString::fromUtf8(source));
    }
    static QString pseudo(const QString &s) {
        static const QHash<QChar, QChar> accents = {
            { u'a', u'á' }, { u'e', u'é' }, { u'i', u'î' }, { u'o', u'ö' }, { u'u', u'ü' }, { u'c', u'ç' }, { u'n', u'ñ' },
            { u'A', u'Å' }, { u'E', u'É' }, { u'I', u'Î' }, { u'O', u'Ö' }, { u'U', u'Ü' }, { u'C', u'Ç' }, { u'N', u'Ñ' },
        };
        QString out;
        int letters = 0;
        for (int i = 0; i < s.size(); ++i) {
            const QChar c = s.at(i);
            if (c == u'%' && i + 1 < s.size()) {
                // %1, %n, %L1 pass unchanged
                out += c;
                while (i + 1 < s.size() && (s.at(i + 1).isDigit() || s.at(i + 1) == u'n' || s.at(i + 1) == u'L')) out += s.at(++i);
                continue;
            }
            if (c.isLetter()) ++letters;
            out += accents.value(c, c);
        }
        return QStringLiteral("[") + out + QString(qMax(1, letters / 3), u'~') + QStringLiteral("]");
    }
};

QString accessibleName(QQuickItem *item) {
    QAccessibleInterface *a = QAccessible::queryAccessibleInterface(item);
    return a ? a->text(QAccessible::Name) : QString();
}

bool shown(QQuickItem *item) {
    for (QQuickItem *i = item; i; i = i->parentItem())
        if (!i->isVisible() || i->opacity() <= 0.0) return false;
    return item->width() > 0 && item->height() > 0;
}

void walk(QQuickItem *root, const std::function<void(QQuickItem *)> &fn) {
    if (!root) return;
    fn(root);
    for (QQuickItem *child : root->childItems()) walk(child, fn);
}

QString describe(QQuickItem *item) {
    const QString name = item->objectName();
    return name.isEmpty() ? QString::fromLatin1(item->metaObject()->className()) : name;
}

struct Totals {
    int controls = 0, unnamed = 0, windows = 0, traps = 0;
    int texts = 0, unmarked = 0, clipped = 0, elided = 0;
    QStringList cut; // each clipped or elided text with its window (behaviour.md §22)
};

// Every control a person reaches with Tab has a name
void checkNames(SmokeDriver &d, const QString &where, QQuickItem *root, Totals &t) {
    QStringList missing;
    int controls = 0;
    walk(root, [&](QQuickItem *item) {
        if (!item->activeFocusOnTab() || !item->isEnabled() || !shown(item)) return;
        ++controls;
        if (accessibleName(item).trimmed().isEmpty()) missing << describe(item);
    });
    t.controls += controls;
    t.unnamed += int(missing.size());
    if (!missing.isEmpty()) std::printf("smoke: qp18: %s: unnamed: %s\n", qPrintable(where), qPrintable(missing.join(QStringLiteral(", "))));
    d.check(QStringLiteral("qp18: %1: every control Tab reaches has a name (%2 controls)").arg(where).arg(controls), missing.isEmpty());
}

// Tab through the window: the focus moves on every press and never stays
// inside one control (a keyboard trap)
void checkTabWalk(SmokeDriver &d, const QString &where, QQuickWindow *w, Totals &t) {
    w->requestActivate();
    d.waitFor(1500, [&]() { return QGuiApplication::focusWindow() == w; });
    if (QGuiApplication::focusWindow() != w) {
        std::printf("smoke: qp18: %s: the window did not take the keyboard, Tab walk skipped\n", qPrintable(where));
        return;
    }
    w->contentItem()->forceActiveFocus();
    QSet<QQuickItem *> seen;
    QQuickItem *last = nullptr;
    int same = 0, trapped = 0;
    QString trapAt;
    for (int i = 0; i < 80; ++i) {
        QTest::keyClick(w, Qt::Key_Tab);
        QQuickItem *now = w->activeFocusItem();
        if (now == last) {
            if (++same >= 2 && !trapped) {
                trapped = 1;
                trapAt = now ? describe(now) : QStringLiteral("(none)");
            }
        } else {
            same = 0;
        }
        last = now;
        if (now) seen.insert(now);
    }
    // Escape out of whatever the walk left open
    QTest::keyClick(w, Qt::Key_Escape);
    // Staying is right where the window has one stop only
    int stops = 0;
    walk(w->contentItem(), [&](QQuickItem *item) { stops += item->activeFocusOnTab() && item->isEnabled() && shown(item); });
    if (stops <= 1) trapped = 0;
    t.traps += trapped;
    ++t.windows;
    if (trapped) std::printf("smoke: qp18: %s: Tab stays on %s\n", qPrintable(where), qPrintable(trapAt));
    // -- WP QPI4 -- for the record (review w4 N6): a cycle short of every stop
    // is not a trap the check above sees
    if (seen.size() < qMin(stops, 80))
        std::printf("smoke: qp18: %s: Tab reached %d of %d stops\n", qPrintable(where), int(seen.size()), stops);
    d.check(QStringLiteral("qp18: %1: Tab moves through %2 stops and is never trapped").arg(where).arg(seen.size()), !trapped && !seen.isEmpty());
}

// The pseudo pass: what is not marked did not go through a translation
void countTexts(const QString &where, QQuickItem *root, Totals &t, QStringList *unmarked) {
    walk(root, [&](QQuickItem *item) {
        if (!shown(item) || qstrcmp(item->metaObject()->className(), "QQuickText") != 0) return;
        const QString text = item->property("text").toString().trimmed();
        if (text.isEmpty()) return;
        ++t.texts;
        bool letters = false;
        for (QChar c : text) letters = letters || (c.isLetter() && c.unicode() < 0x80);
        if (letters && !text.contains(u'[') && text.size() > 2) {
            ++t.unmarked;
            if (unmarked->size() < 2000) unmarked->append(where + QStringLiteral(": ") + text.left(60));
        }
        // Qt::ElideLeft is 0: any mode but ElideNone is an ellipsis by design;
        // truncated without one is a maximumLineCount cut
        if (item->property("truncated").toBool()) {
            const bool elides = item->property("elide").toInt() != Qt::ElideNone;
            ++(elides ? t.elided : t.clipped);
            t.cut.append(QStringLiteral("%1 %2: %3").arg(elides ? QStringLiteral("elided") : QStringLiteral("clipped"), where,
                                                          text.left(60)));
        }
    });
}

// The windows of a pass, each opened in turn: the main window (overview, an
// input and an output page), every registered tool window, every Settings page
struct Stop {
    QString name;
    std::function<QQuickWindow *()> open;
    std::function<void()> close;
};

QList<Stop> stopsOf(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    QQuickWindow *main = d.mainWindow();
    QList<Stop> stops;
    stops.append({ QStringLiteral("main"), [main]() { return main; }, []() {} });
    const int outputChannel = d.app().device->numInputs();
    stops.append({ QStringLiteral("main-input"), [main]() { main->setProperty("selectedChannel", 0); return main; },
                   [main]() { main->setProperty("selectedChannel", -1); } });
    stops.append({ QStringLiteral("main-output"), [main, outputChannel]() { main->setProperty("selectedChannel", outputChannel); return main; },
                   [main]() { main->setProperty("selectedChannel", -1); } });
    for (const Registrar::WindowEntry &entry : Registrar::windows()) {
        const QString id = QString::fromLatin1(entry.action);
        // The firmware window may restart the device; Settings has its pages below
        if (id == QLatin1String("device.firmwareUpdate") || id == QLatin1String("edit.settings") || id.startsWith(QLatin1String("smoke.")))
            continue;
        stops.append({ QString::fromLatin1(entry.type), [shell, id]() { return qobject_cast<QQuickWindow *>(shell->openWindow(id)); },
                       [shell, id]() { if (QWindow *w = shell->openWindow(id)) w->close(); } });
    }
    auto *settings = Registrar::instance<SettingsController>();
    for (const SettingsRegistry::PageEntry &page : SettingsRegistry::pages()) {
        const QString id = QString::fromLatin1(page.id);
        if (page.group == SettingsRegistry::Hidden || id.startsWith(QLatin1String("smoke"))) continue;
        stops.append({ QStringLiteral("settings-") + id,
                       [shell, settings, id]() {
                           auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
                           if (settings) settings->open(id);
                           return (settings && settings->currentPage() == id) ? w : nullptr;
                       },
                       []() {} });
    }
    // -- WP LAY7 -- Modern's pages (their windows are the ones above)
    SmokeDriver *driver = &d;
    for (const QString &key : d.modernTabs()) {
        stops.append({ QStringLiteral("modern-") + QString(key).replace(QLatin1Char('/'), QLatin1Char('-')),
                       [driver, main, key]() -> QQuickWindow * {
                           if (!driver->app().shell->pageHost() && !driver->switchLayout("modern")) return nullptr;
                           return driver->goTo(key) ? main : nullptr;
                       },
                       []() {} });
    }
    return stops;
}

void visit(SmokeDriver &d, const QList<Stop> &stops, const std::function<void(const QString &, QQuickWindow *)> &fn) {
    for (const Stop &s : stops) {
        // -- WP LAY7 -- a window left open keeps its panel off Modern's page
        if (s.name.startsWith(QLatin1String("modern-")) && !d.app().shell->shownWindows().isEmpty())
            for (const QString &action : d.app().shell->shownWindows()) d.app().shell->closeWindow(action);
        QQuickWindow *w = s.open();
        if (!w) d.check(QStringLiteral("qp18: %1 shows").arg(s.name), !s.name.startsWith(QLatin1String("modern-")));
        if (!w || !d.waitFor(2000, [w]() { return w->isVisible(); })) {
            s.close();
            continue;
        }
        d.spin(120);
        fn(s.name, w);
        s.close();
        d.spin(20);
    }
    if (d.app().shell->pageHost()) d.switchLayout("classic");   // -- WP LAY7 --
    if (QWindow *sw = d.app().shell->openWindow(QStringLiteral("edit.settings"))) sw->close();
    d.mainWindow()->requestActivate();
    d.spin(50);
}

// -- WP LAY7 -- What opens over Modern's pages
void modernPopups(SmokeDriver &d, Totals &t) {
    QQuickWindow *main = d.mainWindow();
    d.check("qp18: Modern, for its popups", d.switchLayout("modern"));
    if (!d.app().shell->pageHost()) return;
    QQuickItem *all = main->contentItem();   // the overlay is in it
    const auto press = [&](const char *name) {
        QQuickItem *button = d.item(QString::fromLatin1(name));
        return button && QMetaObject::invokeMethod(button, "clicked");
    };
    const struct {
        const char *name;
        std::function<bool()> open;
    } popups[] = {
        { "palette", [&] { Registrar::instance<PaletteController>()->open(); return true; } },
        { "switcher", [&] { d.app().shell->trigger(QStringLiteral("device.devices")); return true; } },
        { "app menu", [&] { return press("AppMenuButton"); } },
        { "notifications", [&] { return press("TopBar.bell"); } },
        { "AI", [&] { return press("TopBar.ai"); } },
        { "theme", [&] { return press("TopBar.theme"); } },
    };
    main->requestActivate();
    d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == main; });
    int before = 0;
    walk(all, [&](QQuickItem *item) { before += item->isVisible(); });
    for (const auto &p : popups) {
        const bool pressed = p.open();
        int shown = 0;
        const bool open = pressed && d.waitFor(2000, [&] {
            shown = 0;
            walk(all, [&](QQuickItem *item) { shown += item->isVisible(); });
            QQuickItem *drawer = qstrcmp(p.name, "notifications") ? nullptr : d.item(QStringLiteral("NotificationDrawer"));
            return drawer ? drawer->property("open").toBool() : shown > before + 3;
        });
        d.spin(250);
        d.check(QStringLiteral("qp18: Modern's %1 opens").arg(QLatin1String(p.name)), open);
        checkNames(d, QStringLiteral("Modern's ") + QLatin1String(p.name), all, t);
        if (!qstrcmp(p.name, "notifications")) press("TopBar.bell");
        else QTest::keyClick(main, Qt::Key_Escape);
        d.waitFor(2000, [&] {
            int now = 0;
            walk(all, [&](QQuickItem *item) { now += item->isVisible(); });
            return now <= before + 3;
        });
    }
    d.switchLayout("classic");
}

void accessSection(SmokeDriver &d) {
    d.allowSectionTime(240);   // -- WP LAY7 --
    Shell *shell = d.app().shell;
    QDir().mkpath(d.outDir() + QStringLiteral("/light"));
    const Qt::ColorScheme scheme = shell->colorSchemeOverride();
    Totals t;
    visit(d, stopsOf(d), [&](const QString &name, QQuickWindow *w) {
        checkNames(d, name, w->contentItem(), t);
        checkTabWalk(d, name, w, t);
        // -- WP QPI5 -- the light picture without the walk's last focus frame
        // (the root item is a focus scope: focusing it keeps its focused child)
        QQuickItem *focused = w->activeFocusItem();
        if (focused && focused != w->contentItem()) focused->setFocus(false, Qt::OtherFocusReason);
        shell->setColorSchemeOverride(Qt::ColorScheme::Light);
        d.spin(80);
        d.grab(w, QStringLiteral("light/%1.png").arg(name));
        shell->setColorSchemeOverride(scheme);
        d.spin(30);
    });
    modernPopups(d, t);   // -- WP LAY7 --
    std::printf("smoke: qp18: accessibility: %d controls, %d unnamed; %d windows walked, %d traps\n", t.controls, t.unnamed,
                t.windows, t.traps);
}

// behaviour.md §22. Under QP19's pseudo launches (DSPI_SMOKE_PSEUDO=en, then
// =pseudo, each its own run) one half each: the English writes the clipping
// baseline (clipped.txt), the pseudo run compares with it. Run in the main
// pass (no DSPI_SMOKE_PSEUDO) it does both halves in turn.
void pseudoSection(SmokeDriver &d) {
    d.allowSectionTime(240);   // -- WP LAY7 --
    const QByteArray locale = qgetenv("DSPI_SMOKE_PSEUDO");
    const bool english = locale != "pseudo";
    const bool marked = locale != "en";
    const QString dir = locale.isEmpty() ? d.outDir() + QStringLiteral("/pseudo") : d.outDir();
    QDir().mkpath(dir);
    const QList<Stop> stops = stopsOf(d);
    QStringList unmarked;

    int baseline = -1;
    if (english) {
        Totals en;
        visit(d, stops, [&](const QString &name, QQuickWindow *w) {
            countTexts(name, w->contentItem(), en, &unmarked);
            if (!locale.isEmpty()) d.grab(w, QStringLiteral("%1.png").arg(name));
        });
        baseline = en.clipped;
        QFile f(dir + QStringLiteral("/clipped.txt"));
        if (f.open(QIODevice::WriteOnly | QIODevice::Truncate)) f.write(QByteArray::number(baseline) + '\n');
        std::printf("smoke: qp18: English: %d texts, %d clipped, %d elided\n", en.texts, en.clipped, en.elided);
        for (const QString &line : std::as_const(en.cut)) std::printf("smoke: qp18: English %s\n", qPrintable(line));
        unmarked.clear();
    } else {
        QFile f(d.outDir() + QStringLiteral("/../pseudo-en/clipped.txt"));
        if (f.open(QIODevice::ReadOnly)) baseline = f.readAll().trimmed().toInt();
    }
    if (!marked) return;

    PseudoTranslator pseudo;
    QCoreApplication::installTranslator(&pseudo);
    d.app().engine->retranslate();
    d.spin(150);
    QObject *quit = d.object(QStringLiteral("Menu.file.quit"));
    QObject *action = quit ? quit->property("action").value<QObject *>() : nullptr;
    d.check(QStringLiteral("qp18: the pseudo locale reaches the menus"),
            action && action->property("text").toString().startsWith(u'['));
    Totals t;
    visit(d, stops, [&](const QString &name, QQuickWindow *w) {
        d.grab(w, (locale.isEmpty() ? QStringLiteral("pseudo/%1.png") : QStringLiteral("%1.png")).arg(name));
        countTexts(name, w->contentItem(), t, &unmarked);
    });
    // -- WP LAY7 -- Modern's bars in the narrowest window, the texts a third longer
    if (QQuickWindow *main = d.mainWindow(); d.switchLayout("modern")) {
        const QSize was = main->size();
        main->resize(1000, 660);
        d.spin(300);
        for (const char *bar : { "TopBar", "Dock", "PageTabs" })
            d.check(QStringLiteral("qp18: Modern's %1 fits 1000 px in the pseudo locale").arg(QLatin1String(bar)), SmokeDriver::fits(d.item(QString::fromLatin1(bar))));
        d.grab(main, locale.isEmpty() ? QStringLiteral("pseudo/modern-1000.png") : QStringLiteral("modern-1000.png"));
        main->resize(was);
        d.switchLayout("classic");
    }
    QCoreApplication::removeTranslator(&pseudo);
    d.app().engine->retranslate();
    d.spin(150);
    QFile list(dir + QStringLiteral("/unmarked.txt"));
    if (list.open(QIODevice::WriteOnly | QIODevice::Truncate)) list.write(unmarked.join(u'\n').toUtf8() + '\n');
    std::printf("smoke: qp18: pseudo: %d texts, %d not through a translation (unmarked.txt), %d clipped "
                "(%d in English), %d elided\n",
                t.texts, t.unmarked, t.clipped, baseline, t.elided);
    for (const QString &line : std::as_const(t.cut)) std::printf("smoke: qp18: pseudo %s\n", qPrintable(line));
    d.check(QStringLiteral("qp18: the pseudo locale clips no more than English does"), baseline < 0 || t.clipped <= baseline);
    // -- WP LAY7 -- the menu is a new one: the layout was switched meanwhile
    quit = d.object(QStringLiteral("Menu.file.quit"));
    action = quit ? quit->property("action").value<QObject *>() : nullptr;
    d.check(QStringLiteral("qp18: back in English after the pass"),
            action && !action->property("text").toString().startsWith(u'['));
}

// Registered in QP19's pseudo launches where the driver has them, otherwise in
// the main pass
template <class D>
auto registerPseudo(int) -> decltype(D::Mode::Pseudo, SmokeDriver::Section("", 0, nullptr)) {
    return SmokeDriver::Section("qp18-pseudo", 10, &pseudoSection, D::Mode::Pseudo);
}
template <class D>
SmokeDriver::Section registerPseudo(long) {
    return SmokeDriver::Section("qp18-pseudo", 195, &pseudoSection);
}

} // namespace

static const SmokeDriver::Section qp18Access("qp18-access", 190, &accessSection);
static const SmokeDriver::Section qp18Pseudo = registerPseudo<SmokeDriver>(0);
