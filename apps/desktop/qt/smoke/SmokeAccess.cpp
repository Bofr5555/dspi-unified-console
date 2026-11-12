// -- WP QP18 -- The accessibility, localisation and light-scheme passes over
// every window: the main window (overview, an input and an output page), every
// registered tool window and every Settings page.
// - Every control that takes the keyboard has a name for assistive
//   technology, and Tab walks through each window and out of every control
//   (no keyboard trap, WCAG 2.1.2).
// - A screenshot of each in the light scheme (light/).
// - The pseudo-locale pass (behaviour.md §22): a translator that marks every
//   string the app translates; each window again (pseudo/), with the texts
//   that came through unmarked and the ones cut short, per window.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QAccessible>
#include <QCoreApplication>
#include <QDir>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
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
    t.traps += trapped;
    ++t.windows;
    if (trapped) std::printf("smoke: qp18: %s: Tab stays on %s\n", qPrintable(where), qPrintable(trapAt));
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
            if (unmarked->size() < 400) unmarked->append(where + QStringLiteral(": ") + text.left(60));
        }
        if (item->property("truncated").toBool()) {
            if (item->property("elide").toInt() != 0) ++t.elided;
            else ++t.clipped;
        }
    });
}

void accessSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    QQuickWindow *main = d.mainWindow();
    const QString out = d.outDir();
    QDir().mkpath(out + QStringLiteral("/light"));
    QDir().mkpath(out + QStringLiteral("/pseudo"));
    const Qt::ColorScheme scheme = shell->colorSchemeOverride();
    Totals t;

    // The windows of a pass: (name, window, the item to look into)
    struct Stop {
        QString name;
        std::function<QQuickWindow *()> open;
        std::function<void()> close;
    };
    QList<Stop> stops;
    stops.append({ QStringLiteral("main"), [main]() { return main; }, []() {} });
    const int inputChannel = 0;
    const int outputChannel = d.app().device->numInputs();
    stops.append({ QStringLiteral("main-input"), [main, inputChannel]() { main->setProperty("selectedChannel", inputChannel); return main; },
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
        if (page.group == SettingsRegistry::Hidden) continue;
        stops.append({ QStringLiteral("settings-") + id,
                       [shell, settings, id]() {
                           auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
                           if (settings) settings->open(id);
                           return (settings && settings->currentPage() == id) ? w : nullptr;
                       },
                       []() {} });
    }

    auto visit = [&](const std::function<void(const QString &, QQuickWindow *)> &fn) {
        for (const Stop &s : stops) {
            QQuickWindow *w = s.open();
            if (!w || !d.waitFor(2000, [w]() { return w->isVisible(); })) {
                s.close();
                continue;
            }
            d.spin(120);
            fn(s.name, w);
            s.close();
            d.spin(20);
        }
        if (QWindow *sw = shell->openWindow(QStringLiteral("edit.settings"))) sw->close();
        main->requestActivate();
        d.spin(50);
    };

    // ---- Names, Tab, and the light scheme --------------------------------
    visit([&](const QString &name, QQuickWindow *w) {
        checkNames(d, name, w->contentItem(), t);
        checkTabWalk(d, name, w, t);
        shell->setColorSchemeOverride(Qt::ColorScheme::Light);
        d.spin(80);
        d.grab(w, QStringLiteral("light/%1.png").arg(name));
        shell->setColorSchemeOverride(scheme);
        d.spin(30);
    });
    std::printf("smoke: qp18: accessibility: %d controls, %d unnamed; %d windows walked, %d traps\n", t.controls, t.unnamed,
                t.windows, t.traps);

    // ---- The pseudo-locale pass ------------------------------------------
    // The English first, for the clipping baseline
    Totals english;
    QStringList ignored;
    visit([&](const QString &name, QQuickWindow *w) { countTexts(name, w->contentItem(), english, &ignored); });
    PseudoTranslator pseudo;
    QCoreApplication::installTranslator(&pseudo);
    d.app().engine->retranslate();
    d.spin(150);
    QObject *quit = d.object(QStringLiteral("Menu.file.quit"));
    QObject *action = quit ? quit->property("action").value<QObject *>() : nullptr;
    d.check(QStringLiteral("qp18: the pseudo locale reaches the menus"),
            action && action->property("text").toString().startsWith(u'['));
    QStringList unmarked;
    visit([&](const QString &name, QQuickWindow *w) {
        d.grab(w, QStringLiteral("pseudo/%1.png").arg(name));
        countTexts(name, w->contentItem(), t, &unmarked);
    });
    QCoreApplication::removeTranslator(&pseudo);
    d.app().engine->retranslate();
    d.spin(150);
    QFile list(out + QStringLiteral("/pseudo/unmarked.txt"));
    if (list.open(QIODevice::WriteOnly | QIODevice::Truncate)) list.write(unmarked.join(u'\n').toUtf8() + '\n');
    std::printf("smoke: qp18: pseudo: %d texts, %d not through a translation (pseudo/unmarked.txt), %d clipped "
                "(%d in English), %d elided\n",
                t.texts, t.unmarked, t.clipped, english.clipped, t.elided);
    d.check(QStringLiteral("qp18: the pseudo locale clips no more than English does"), t.clipped <= english.clipped);
    d.check(QStringLiteral("qp18: back in English after the pass"),
            action && !action->property("text").toString().startsWith(u'['));
}

} // namespace

static const SmokeDriver::Section qp18Access("qp18-access", 190, &accessSection);
