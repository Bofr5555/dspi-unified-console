// -- WP QTPOP -- Every tool window a menu row opens, measured on the demo: its
// scrolling bodies (the visible Flickables, found in its item tree, whatever
// the window declares) and whether one scrolls although its content would fit
// within the cap (the main window's size and the screen's, less 48 px each
// side). Lists of records (a ListView: events, history, snapshots...) scroll
// by design and are reported, not checked. Each window is measured with the
// main window at its default size and, on a screen launch, with the main
// window filling the screen; one line per window goes to the log
// ("smoke: qtpop: ...") and one PNG to <out>/popups/.
//
// The main run measures on the offscreen platform's own screen; run.sh's
// popups-1366x768 and popups-1920x1080 launches (DSPI_SMOKE_POPUPS) give the
// offscreen platform that screen and also check that the heavy effect windows
// show no vertical scroll bar at 1920x1080.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QDir>
#include <QGuiApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QScreen>
#include <QWindow>
#include <algorithm>
#include <cmath>
#include <cstdio>

namespace {

constexpr int kMargin = 48;   // Shell::fitCap's

// The heavy windows that must not scroll on a 1920x1080 screen, with the main
// window at its default size or filling the screen. The leveller (380 px wide,
// 730 tall) and the upmixer (400 wide, 1757 tall) need more than the default
// size's cap and do scroll there: the rule's check covers them.
const QStringList kHeavy = { QStringLiteral("CrossfeedWindow"), QStringLiteral("TubeWindow"),
                             QStringLiteral("SubharmonicWindow"), QStringLiteral("PsybassWindow"),
                             QStringLiteral("LoudnessWindow"), QStringLiteral("SiggenWindow") };
// Fitted by a rule of its own: the matrix to its table within the screen (QPI5)
const QStringList kOwnFit = { QStringLiteral("MatrixWindow") };
// Offered nowhere yet (SmokeGallery's kHidden)
const QStringList kHidden = { QStringLiteral("window.mode") };

struct Measure {
    QSize size;
    int wantW = 0, wantH = 0;   // the window's size at which no body scrolls
    bool scrollsV = false, scrollsH = false;
    bool listScrolls = false;   // a list of records scrolls
    int needH = -1;             // the height its bodies need (less than the window's when they are short)
};

bool shown(QQuickItem *it) {
    for (; it; it = it->parentItem())
        if (!it->isVisible() || it->opacity() == 0) return false;
    return true;
}

void collect(QQuickItem *it, QList<QQuickItem *> &out) {
    if (!it || !it->isVisible()) return;
    if (it->inherits("QQuickFlickable")) out << it;
    for (QQuickItem *child : it->childItems()) collect(child, out);
}

Measure measure(QQuickWindow *w) {
    Measure m;
    m.size = w->size();
    m.wantW = m.size.width();
    m.wantH = m.size.height();
    QList<QQuickItem *> bodies;
    collect(w->contentItem(), bodies);
    for (QQuickItem *b : bodies) {
        // A combo box's list, a one-line legend: not a body
        if (b->height() < 60 || !shown(b)) continue;
        const qreal ch = b->property("contentHeight").toReal(), cw = b->property("contentWidth").toReal();
        const qreal moreH = ch - b->height(), moreW = cw - b->width();
        if (b->inherits("QQuickListView")) {
            m.listScrolls = m.listScrolls || moreH > 1;
            continue;
        }
        m.needH = qMax(m.needH, int(std::ceil(m.size.height() + moreH)));
        if (moreH > 1) {
            m.scrollsV = true;
            m.wantH = qMax(m.wantH, int(std::ceil(m.size.height() + moreH)));
        }
        if (moreW > 1) {
            m.scrollsH = true;
            m.wantW = qMax(m.wantW, int(std::ceil(m.size.width() + moreW)));
        }
    }
    return m;
}

QSize cap(QQuickWindow *main, QWindow *w) {
    QSize room = w->screen() ? w->screen()->availableGeometry().size() : QSize(100000, 100000);
    if (main && main->isVisible()) room = room.boundedTo(main->size());
    return QSize(qMax(0, room.width() - 2 * kMargin), qMax(0, room.height() - 2 * kMargin));
}

QString shortName(const QString &type) {
    QString name = type;
    if (name.endsWith(QLatin1String("Window"))) name.chop(6);
    return name.toLower();
}

// Measures the window `quick` (named `name`) in phase `phase`, logs it and
// checks the rule; `heavy` adds the no-scroll check
void judge(SmokeDriver &d, QQuickWindow *main, QQuickWindow *quick, const QString &name, const QString &phase, bool heavy) {
    d.waitIdle();
    d.spin(400);
    const Measure m = measure(quick);
    const QSize c = cap(main, quick);
    const bool needlessV = m.scrollsV && m.wantH <= c.height();
    const bool needlessH = m.scrollsH && m.wantW <= c.width();
    std::printf("smoke: qtpop: %s %s: %dx%d, wants %dx%d (needs %d), cap %dx%d, scrolls %s%s%s%s\n", qPrintable(phase),
                qPrintable(name), m.size.width(), m.size.height(), m.wantW, m.wantH, m.needH, c.width(), c.height(),
                m.scrollsV ? "V" : "", m.scrollsH ? "H" : "", !m.scrollsV && !m.scrollsH ? "no" : "",
                m.listScrolls ? " (a list scrolls)" : "");
    std::fflush(stdout);
    d.grab(quick, QStringLiteral("popups/%1-%2.png").arg(phase, name));
    d.check(QStringLiteral("qtpop: %1 %2 scrolls only when its content does not fit in %3x%4")
                .arg(phase, name).arg(c.width()).arg(c.height()),
            !needlessV && !needlessH);
    // Grown no further than the cap (a window's minimum and default size may be more)
    const int mostW = std::max({ c.width(), quick->minimumWidth(), quick->property("defaultWidth").toInt() });
    const int mostH = std::max({ c.height(), quick->minimumHeight(), quick->property("defaultHeight").toInt() });
    if (!kOwnFit.contains(quick->objectName()))
        d.check(QStringLiteral("qtpop: %1 %2 grows no larger than the cap").arg(phase, name),
                m.size.width() <= mostW && m.size.height() <= mostH);
    // Grown past what its content needs (beyond its default size)
    if (!kOwnFit.contains(quick->objectName())) {
        const int base = qMax(quick->minimumHeight(), quick->property("defaultHeight").toInt());
        d.check(QStringLiteral("qtpop: %1 %2 is no taller than its content needs").arg(phase, name),
                m.needH < 0 || m.size.height() <= qMax(base, m.needH) + 2);
    }
    if (heavy) d.check(QStringLiteral("qtpop: %1 %2 shows no vertical scroll bar").arg(phase, name), !m.scrollsV);
}

void measureAll(SmokeDriver &d, const QString &phase, bool checkHeavy) {
    Shell *shell = d.app().shell;
    QQuickWindow *main = d.mainWindow();
    auto *settings = Registrar::instance<SettingsController>();
    for (const Registrar::WindowEntry &entry : Registrar::windows()) {
        const QString type = QString::fromLatin1(entry.type);
        const QString action = QString::fromLatin1(entry.action);
        if (type.startsWith(QLatin1String("Smoke")) || kHidden.contains(action)) continue;
        QWindow *window = nullptr;
        const bool open = d.waitFor(3000, [&] {
            window = shell->openWindow(action);
            return window && window->isVisible();
        });
        auto *quick = qobject_cast<QQuickWindow *>(window);
        d.check(QStringLiteral("qtpop: %1 opens").arg(type), open && quick);
        if (!open || !quick) continue;
        // -- WP INT1 -- Judge the fit, not an earlier section's hand size:
        // lkui1-system and atqt-units resize the System window by hand for
        // their screenshots, and a size the person gives a window is kept
        // (QTPOP2, and ToolWindow's Settings keep it across showings), so it
        // may be past the cap. Shown again from its default size, as a first
        // opening is: resized while it shows (a showing's hand size ends with
        // it), so its layout has followed before it closes.
        const QSize first(quick->property("defaultWidth").toInt(), quick->property("defaultHeight").toInt());
        if (!first.isEmpty()) window->resize(first);
        d.spin(200);
        window->close();
        d.spin(100);
        const bool again = d.waitFor(3000, [&] {
            window = shell->openWindow(action);
            return window && window->isVisible();
        });
        d.check(QStringLiteral("qtpop: %1 opens again from its default size").arg(type), again && window == quick);
        if (!again || window != quick) continue;
        const bool heavy = checkHeavy && kHeavy.contains(type);
        if (type == QLatin1String("SettingsWindow") && settings) {
            for (const QVariant &row : settings->sidebarRows(QString())) {
                const QString page = row.toMap().value(QStringLiteral("id")).toString();
                if (page.isEmpty()) continue;
                settings->open(page);
                judge(d, main, quick, QStringLiteral("settings-%1").arg(page), phase, false);
            }
        } else if (type == QLatin1String("MeasureWindow")) {
            QQuickItem *tabs = d.item(QStringLiteral("Measure.tabs"), quick->contentItem());
            const int count = tabs ? tabs->property("count").toInt() : 0;
            for (int i = 0; i < count; ++i) {
                QQuickItem *button = nullptr;
                QMetaObject::invokeMethod(tabs, "itemAt", Q_RETURN_ARG(QQuickItem *, button), Q_ARG(int, i));
                if (!button || !button->isVisible() || !button->isEnabled()) continue;
                tabs->setProperty("currentIndex", i);
                judge(d, main, quick, QStringLiteral("measure-%1").arg(i), phase, false);
            }
            if (tabs) tabs->setProperty("currentIndex", 0);
        } else {
            judge(d, main, quick, shortName(type), phase, heavy);
        }
        window->close();
        d.spin(100);
    }
}

// QTPOP2: a size the person gives a fitted window sticks, larger and smaller;
// content that grows while it shows still refits it, even after a manual
// resize, and never shrinks a window the person enlarged; Settings grows to a
// long page on a big screen (the main window filling it)
QWindow *openTool(SmokeDriver &d, const char *action) {
    QWindow *window = nullptr;
    d.waitFor(3000, [&] {
        window = d.app().shell->openWindow(QString::fromLatin1(action));
        return window && window->isVisible();
    });
    d.waitIdle();
    d.spin(400);
    return window;
}

void resizeChecks(SmokeDriver &d) {
    QWindow *cf = openTool(d, "tools.crossfeed");
    d.check("qtpop2: the crossfeed window opens", cf != nullptr);
    if (cf) {
        const int w = cf->width(), h0 = cf->height();
        cf->resize(w, h0 + 100);   // as a drag of its corner would
        d.spin(400);
        d.check(QStringLiteral("qtpop2: crossfeed resized by hand to %1 stays (is %2)").arg(h0 + 100).arg(cf->height()),
                cf->height() == h0 + 100);
        const int smaller = qMax(cf->minimumHeight(), h0 - 60);
        cf->resize(w, smaller);
        d.spin(400);
        d.check(QStringLiteral("qtpop2: crossfeed resized by hand to %1 stays (is %2)").arg(smaller).arg(cf->height()),
                cf->height() == smaller);
        cf->resize(w + 80, smaller);   // wider: the captions re-flow, which is no reason to refit
        d.spin(400);
        d.check(QStringLiteral("qtpop2: crossfeed widened by hand stays %1x%2 (is %3x%4)")
                    .arg(w + 80).arg(smaller).arg(cf->width()).arg(cf->height()),
                cf->width() == w + 80 && cf->height() == smaller);
        cf->resize(w, h0);
        cf->close();
        d.spin(100);
        cf = openTool(d, "tools.crossfeed");
        d.check(QStringLiteral("qtpop2: opened again, crossfeed fits its content again (%1, was %2)").arg(cf ? cf->height() : -1).arg(h0),
                cf && cf->height() == h0);
        if (cf) cf->close();
        d.spin(100);
    }

    auto *settings = Registrar::instance<SettingsController>();
    QWindow *sw = settings ? openTool(d, "edit.settings") : nullptr;
    d.check("qtpop2: the Settings window opens", sw && sw->isVisible());
    if (!sw || !settings) return;
    auto *quick = qobject_cast<QQuickWindow *>(sw);
    const QSize cap = d.app().shell->fitCap(sw);
    settings->open(QStringLiteral("updates"));
    d.spin(400);
    const int shortH = sw->height();
    settings->open(QStringLiteral("networking"));
    d.spin(400);
    const Measure grown = measure(quick);
    std::printf("smoke: qtpop2: settings updates %d, networking %d (needs %d, cap %d)\n", shortH, sw->height(), grown.needH, cap.height());
    std::fflush(stdout);
    d.check(QStringLiteral("qtpop2: Settings grows from %1 to the longer page (%2, needs %3) while it shows")
                .arg(shortH).arg(sw->height()).arg(grown.needH),
            sw->height() > shortH && (sw->height() >= grown.needH || sw->height() >= cap.height()));
    d.check("qtpop2: on a big screen Settings > Networking shows without a scroll bar", !grown.scrollsV || grown.needH > cap.height());
    const int taller = sw->height() + 15;
    sw->resize(sw->width(), taller);
    d.spin(400);
    d.check(QStringLiteral("qtpop2: Settings resized by hand to %1 stays (is %2)").arg(taller).arg(sw->height()), sw->height() == taller);
    settings->open(QStringLiteral("advanced"));
    d.spin(400);
    d.check(QStringLiteral("qtpop2: a shorter page does not shrink Settings the person enlarged (%1, is %2)").arg(taller).arg(sw->height()),
            sw->height() == taller);
    settings->open(QStringLiteral("about"));
    d.spin(400);
    d.check(QStringLiteral("qtpop2: a longer page still grows it after the manual resize, to the cap (%1, cap %2)")
                .arg(sw->height()).arg(cap.height()),
            sw->height() == qMax(taller, cap.height()));
    const int small = qMax(sw->minimumHeight(), 640);
    sw->resize(sw->width(), small);
    d.spin(400);
    d.check(QStringLiteral("qtpop2: Settings resized by hand to %1 stays (is %2)").arg(small).arg(sw->height()), sw->height() == small);
    sw->close();
    d.spin(100);
}

void prepare(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("qtpop: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    QDir(d.outDir()).mkpath(QStringLiteral("popups"));
}

// The main run: the offscreen platform's screen, the main window as it is
void mainSection(SmokeDriver &d) {
    prepare(d);
    const QScreen *screen = QGuiApplication::primaryScreen();
    std::printf("smoke: qtpop: screen %dx%d\n", screen->availableGeometry().width(), screen->availableGeometry().height());
    measureAll(d, QStringLiteral("main"), false);
}

// A screen launch: the main window at its default size, then filling the screen
void screenSection(SmokeDriver &d) {
    prepare(d);
    QQuickWindow *main = d.mainWindow();
    const QString screen = SmokeDriver::modeValue();
    const QRect avail = QGuiApplication::primaryScreen()->availableGeometry();
    std::printf("smoke: qtpop: screen %dx%d\n", avail.width(), avail.height());
    d.check(QStringLiteral("qtpop: the screen is %1").arg(screen),
            QStringLiteral("%1x%2").arg(avail.width()).arg(avail.height()) == screen);
    const bool big = avail.width() >= 1920 && avail.height() >= 1080;
    main->resize(1100, 800);   // MainWindow.qml's size
    d.spin(300);
    measureAll(d, QStringLiteral("%1-default").arg(screen), big);
    main->setGeometry(avail);
    d.spin(300);
    measureAll(d, QStringLiteral("%1-full").arg(screen), big);
    if (big) resizeChecks(d);   // -- QTPOP2 --
}

const SmokeDriver::Section popups("qtpop-popups", 880, &mainSection);
const SmokeDriver::Section popupsScreen("qtpop-popups-screen", 10, &screenSection, SmokeDriver::Mode::Popups);

} // namespace
