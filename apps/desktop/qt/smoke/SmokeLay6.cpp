// -- WP LAY6 -- The command palette in Modern, on the demo device. lay6-palette:
// Ctrl+K and the top bar's search open it, Ctrl+K and Escape close it; every
// MenuTable row, section, page, channel and Settings page is in it, with its
// shortcut; it finds by the start, the inside, the first letters and with
// letters left out; what is chosen by Enter or a click runs, the Settings
// window (RB), another device and the A/B compare among them; greyed rows do
// not; under it no shortcut works, a bare key types; from another window; not
// while a dialog asks, not in Classic; the Interrupt Monitor keeps its lines.
#include "SmokeDriver.h"

#include "AutoEqController.h"
#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceInfoController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "HistoryController.h"
#include "LayoutController.h"
#include "MonitorController.h"
#include "OnboardingController.h"
#include "PaletteController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "ShortcutController.h"
#include "SidebarController.h"
#include "SnapshotsController.h"

#include "CoreApi.h"

#include <QAbstractItemModel>
#include <QCoreApplication>
#include <QDir>
#include <QGuiApplication>
#include <QJSValue>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTest>

namespace {

struct Palette {
    SmokeDriver &d;
    Shell *shell;
    LayoutController *layout;
    SettingsController *settings;
    SidebarController *sc;
    PaletteController *pc;
    QQuickWindow *main;

    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay6: ") + QString::fromUtf8(what), ok); }
    void check(const QString &what, bool ok) const { d.check(QStringLiteral("lay6: ") + what, ok); }
    void settle() const {
        d.spin(60);
        d.waitIdle();
        d.spin(60);
    }
    QObject *popup() const { return d.object(QStringLiteral("CommandPalette")); }
    QQuickItem *field() const { return d.item(QStringLiteral("CommandPalette.field")); }
    QQuickItem *list() const { return d.item(QStringLiteral("CommandPalette.list")); }
    bool opened() const { return popup() && popup()->property("opened").toBool(); }
    bool opens() const { return d.waitFor(1500, [&] { return opened(); }); }
    // Closed, or gone with its shell
    bool closes() const { return d.waitFor(1500, [&] { return !popup() || !popup()->property("visible").toBool(); }); }
    void activate(QWindow *w) const {
        w->requestActivate();
        d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == w; });
    }
    void key(Qt::Key k, Qt::KeyboardModifiers mods = Qt::NoModifier) const {
        QTest::keyClick(main, k, mods);
        d.spin(40);
    }
    void ctrlK() const {
        activate(main);
        key(Qt::Key_K, Qt::ControlModifier);
    }
    bool open() const {
        if (!opened()) ctrlK();
        return opens();
    }
    void close() const {
        if (QObject *p = popup()) QMetaObject::invokeMethod(p, "close");
        closes();
    }
    // Typed as the keyboard does, over what is there
    void type(const QString &text) const {
        key(Qt::Key_A, Qt::ControlModifier);
        key(Qt::Key_Backspace);
        for (const QChar c : text) QTest::keyClick(main, c.toLatin1());
        d.spin(60);
    }
    QVariantList rows() const {
        const QVariant v = popup() ? popup()->property("rows") : QVariant();
        return v.canConvert<QJSValue>() ? v.value<QJSValue>().toVariant().toList() : v.toList();
    }
    QVariantMap row(const QString &id) const {
        for (const QVariant &v : rows())
            if (v.toMap().value(QStringLiteral("id")).toString() == id) return v.toMap();
        return {};
    }
    QStringList ids() const {
        QStringList out;
        for (const QVariant &v : rows()) out << v.toMap().value(QStringLiteral("id")).toString();
        return out;
    }
    int current() const { return list() ? list()->property("currentIndex").toInt() : -1; }
    QString currentId() const { return ids().value(current()); }
    QQuickItem *rowItem(const QString &id) const { return d.item(QStringLiteral("CommandPalette.row.") + id); }
    // What a row shows: "title", "where" or "keys"
    QString shownText(const QString &id, const char *part) const {
        QQuickItem *i = d.item(QStringLiteral("CommandPalette.row.%1.%2").arg(id, QLatin1String(part)));
        return i && i->isVisible() ? i->property("text").toString() : QString();
    }
    bool click(QQuickItem *i) const {
        if (!i || !i->isVisible() || i->width() <= 0) return false;
        QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint());
        d.spin(80);
        return true;
    }
    // Opens, types and chooses the first row by Enter
    bool runTyped(const QString &text, const QString &id) const {
        if (!open()) return false;
        type(text);
        if (currentId() != id) {
            qInfo("smoke: lay6: \"%s\" found %s first", qPrintable(text), qPrintable(currentId()));
            return false;
        }
        key(Qt::Key_Return);
        const bool ok = closes();
        settle();
        return ok;
    }
    QString at() const {
        QObject *h = shell->pageHost();
        return h ? h->property("section").toString() + QLatin1Char('/') + h->property("tab").toString() : QString();
    }
    // -- WP INT5 -- The page itself, whichever of its panel's parts shows (LAY5b)
    QString pageKey() const { return shell->pageHost() ? shell->pageHost()->property("pageKey").toString() : QString(); }
    void go(const char *section, const char *tab) const {
        QVariant ok;
        QMetaObject::invokeMethod(shell->pageHost(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QString::fromLatin1(section)),
                                  Q_ARG(QVariant, QString::fromLatin1(tab)));
        settle();
    }
    QJsonObject exported() const {
        QVariant out;
        QMetaObject::invokeMethod(d.app().engine->singletonInstance<QObject *>("UConsole", "MenuTable"), "exported", Q_RETURN_ARG(QVariant, out));
        return QJsonDocument::fromJson(out.toString().toUtf8()).object();
    }
    QVariantList sections() const {
        QObject *table = d.app().engine->singletonInstance<QObject *>("UConsole", "SectionTable");
        return table->property("sections").value<QJSValue>().toVariant().toList();
    }
};

bool switchTo(Palette &p, const QString &layout, const char *shellName) {
    p.layout->setLayout(layout);
    QObject *loader = p.d.object(QStringLiteral("MainWindow.shell"));
    const bool ok = p.d.waitFor(3000, [&] {
        QObject *now = loader ? loader->property("item").value<QObject *>() : nullptr;
        return now && now->objectName() == QLatin1String(shellName);
    });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    p.settle();
    return ok;
}

// ---- The search itself, on entries of its own ----------------------------------

void searchPart(Palette &p) {
    const auto entry = [](const QString &title, const QStringList &keywords = {}, const QStringList &aliases = {}, bool enabled = true) {
        return QVariantMap{ { QStringLiteral("title"), title }, { QStringLiteral("keywords"), keywords },
                            { QStringLiteral("aliases"), aliases }, { QStringLiteral("enabled"), enabled } };
    };
    const auto score = [&](const char *title, const char *query) {
        return PaletteController::score(entry(QString::fromUtf8(title)), QString::fromUtf8(query));
    };
    p.check("the start of the title before the start of a word, before inside one",
            score("Volume Leveller", "vol") > score("Master Volume", "vol") && score("Master Volume", "vol") > score("Convolution", "vol")
                && score("Convolution", "vol") > 0);
    p.check("inside a word before first letters, before letters left out",
            score("Asgard", "sg") > score("Signal Generator", "sg") && score("Signal Generator", "sg") > 0
                && score("Signal Generator", "sg") > score("Headphone Crossfeed", "cfeed") && score("Headphone Crossfeed", "cfeed") > 0);
    p.check("letters left out start at a word and stay near", score("Headphone Crossfeed", "rsfeed") <= 0
            && score("Matrix Mixer", "mtrx") > 0 && score("Matrix Mixer", "mxzz") <= 0);
    p.check("every word, in any order, is in it", score("Import Filters", "fil imp") > 0 && score("Import Filters", "imp zzz") <= 0);
    p.check("case and accents do not count", score("Égaliseur", "EGAL") > 0 && score("Kök", "kok") > 0 && score("Kok", "KÖK") > 0);
    p.check("one letter is found at the start of words only", score("Mute", "m") > 0 && score("System", "m") <= 0);
    QString by;
    const double alias = PaletteController::score(entry(QStringLiteral("Commit"), {}, { QStringLiteral("Save to Device") }), QStringLiteral("save"), &by);
    p.check("an alias is another title", alias > 90 && by == QLatin1String("Save to Device"));
    const double word = PaletteController::score(entry(QStringLiteral("Graphing"), { QStringLiteral("Line Width") }), QStringLiteral("line width"), &by);
    p.check("the keyword with every word is said",
            PaletteController::score(entry(QStringLiteral("Graphing"), { QStringLiteral("Graph Line Glow"), QStringLiteral("Line Width") }),
                                     QStringLiteral("line width"), &by) > 0 && by == QLatin1String("Line Width"));
    p.check("a mark alone does not count beside words", score("Out 8 · None", "8 ·") > 0 && score("Out 8", "out ·") > 0 && score("Out 8 · None", "·") > 0);
    p.check("a keyword counts less, and says itself", word > 0 && word < alias && by == QLatin1String("Line Width"));
    p.check("nothing typed finds all", PaletteController::score(entry(QStringLiteral("X")), QStringLiteral("  ")) > 0);
    const QVariantList list{ entry(QStringLiteral("Undo"), {}, {}, false), entry(QStringLiteral("Undock")), entry(QStringLiteral("Zoom")) };
    const QVariantList all = p.pc->search(list, QString()), some = p.pc->search(list, QStringLiteral("und"));
    p.check("nothing typed keeps the order, a greyed one comes after its equals",
            all.size() == 3 && all.first().toMap().value(QStringLiteral("title")) == QLatin1String("Undo") && some.size() == 2
                && some.first().toMap().value(QStringLiteral("title")) == QLatin1String("Undock"));
}

// ---- Opening and closing ----------------------------------------------------------

void openPart(Palette &p) {
    p.go("overview", "overview");
    QQuickItem *before = p.main->activeFocusItem();
    p.ctrlK();
    p.check("Ctrl+K opens it", p.opens());
    QQuickItem *field = p.field();
    p.check("its field has the keyboard, and is empty", field && field->hasActiveFocus() && field->property("text").toString().isEmpty());
    p.check("the first row that can run is the current one", p.current() == 0 || (p.current() > 0 && p.rows().value(p.current()).toMap().value(QStringLiteral("enabled")).toBool()));
    p.key(Qt::Key_K, Qt::ControlModifier);
    p.check("Ctrl+K closes it again", p.closes());
    p.check("the focus is back where it was", p.main->activeFocusItem() == before);
    p.check("nothing opened by itself", p.at() == QLatin1String("overview/overview"));
    p.open();
    p.key(Qt::Key_Escape);
    p.check("Escape closes it", p.closes());
    p.check("the top bar's search opens it", p.click(p.d.item(QStringLiteral("TopBar.search"))) && p.opens());
    p.check("the Settings are not opened for that", p.shell->pageAction() != QLatin1String("edit.settings"));
    p.type(QStringLiteral("tube"));
    p.close();
    p.open();
    p.check("it opens empty each time", p.field()->property("text").toString().isEmpty() && p.rows().size() > 100);
    QTest::mouseClick(p.main, Qt::LeftButton, Qt::NoModifier, QPoint(20, p.main->height() - 20));
    p.check("a click beside it closes it, and is not the page's", p.closes());
    QQuickItem *keys = p.d.item(QStringLiteral("TopBar.search.keys"));
    p.check("the top bar's search says its shortcut", keys && keys->isVisible() && keys->property("text") == QLatin1String("Ctrl+K"));

    // While a text field has the keyboard
    p.go("settings", "settings");
    QQuickItem *search = p.d.item(QStringLiteral("Settings.search"));
    if (search) search->forceActiveFocus();
    p.d.spin(60);
    p.check("a field has the keyboard", search && search->hasActiveFocus());
    p.ctrlK();
    p.check("Ctrl+K opens it from a text field too", p.opens() && search && search->property("text").toString().isEmpty());
    p.close();
    p.check("the field has the keyboard again", search && search->hasActiveFocus());
    if (search) search->setFocus(false);
    p.go("overview", "overview");
}

// ---- What is in it ------------------------------------------------------------------

// -- WP UFW -- Row `i`'s title in Modern (ChannelInfo.titleOf, UXP A1): its name, then an
// output's number unless the name starts with it ("Left woofer · 1", "Out 1 · L")
QString ufwTitle(ChannelListModel *channels, int i) {
    const QString label = channels->get(i, QStringLiteral("label")).toString();
    const QString name = channels->get(i, QStringLiteral("displayName")).toString().isEmpty() ? label : channels->get(i, QStringLiteral("displayName")).toString();
    if (channels->get(i, QStringLiteral("kind")).toString() != QLatin1String("output")) return name;
    const QString n = QString::number(channels->get(i, QStringLiteral("number")).toInt() + 1);
    const QRegularExpressionMatch own = QRegularExpression(QStringLiteral("^(?:Out )?(\\d+)(?!\\d)")).match(name);
    return own.hasMatch() && own.captured(1) == n ? name : name + QStringLiteral(" · ") + n;
}

void contentsPart(Palette &p) {
    p.open();
    const QStringList ids = p.ids();
    p.check("no entry twice", QSet<QString>(ids.cbegin(), ids.cend()).size() == ids.size());
    const QJsonArray actions = p.exported().value(QLatin1String("actions")).toArray();
    int seen = 0;
    for (const QJsonValue &v : actions) {
        const QJsonObject a = v.toObject();
        const QString id = a.value(QLatin1String("id")).toString();
        const bool hidden = (a.value(QLatin1String("optional")).toBool() && !p.shell->handles(id))
            || (a.contains(QLatin1String("hiddenOn")) && a.value(QLatin1String("hiddenOn")).toString() == p.d.app().device->platformName());
        const QVariantMap row = p.row(id);
        p.check(QStringLiteral("%1 is in it, with its shortcut").arg(id),
                hidden ? row.isEmpty() : row.value(QStringLiteral("keys")).toString() == a.value(QLatin1String("shortcut")).toString() && !row.isEmpty());
        p.check(QStringLiteral("no row has the palette's shortcut (%1)").arg(id), a.value(QLatin1String("shortcut")).toString() != PaletteController::shortcut());
        ++seen;
    }
    p.check("the whole table", seen >= 67);
    p.check("and the way back to Classic", ids.contains(QStringLiteral("app.classicLayout")));
    for (const QVariant &v : p.sections()) {
        const QString id = v.toMap().value(QStringLiteral("id")).toString();
        const bool own = id == QLatin1String("measure") || id == QLatin1String("settings");   // their only page is a row of the same name
        p.check(QStringLiteral("section %1 is in it").arg(id),
                ids.contains(QStringLiteral("section:") + id) || (own && ids.contains(id == QLatin1String("measure") ? QStringLiteral("tools.measure") : QStringLiteral("edit.settings"))));
    }
    for (const char *page : { "equaliser/peq", "equaliser/crossover", "equaliser/delay" })
        p.check(QStringLiteral("page %1 is in it").arg(QLatin1String(page)), ids.contains(QStringLiteral("page:") + QLatin1String(page)));
    // -- WP INT5 -- the parts of LAY5b's panels, those there now
    for (const char *page : { "measure/results", "measure/tune", "system/clock", "settings/control" })
        p.check(QStringLiteral("the part %1 is in it").arg(QLatin1String(page)), ids.contains(QStringLiteral("page:") + QLatin1String(page)));
    p.check("a tab that is not there now is not", !ids.contains(QStringLiteral("page:measure/passive")));
    int pages = 0;
    for (const QVariant &v : p.settings->sidebarRows(QString())) {
        const QString id = v.toMap().value(QStringLiteral("id")).toString();
        if (id.isEmpty()) continue;
        ++pages;
        p.check(QStringLiteral("settings page %1 is in it, by its title").arg(id),
                p.row(QStringLiteral("settings:") + id).value(QStringLiteral("title")).toString() == p.settings->pageTitle(id));
    }
    p.check("the settings pages", pages >= 10);
    ChannelListModel *channels = p.d.app().device->channels();
    bool every = channels->rowCount() > 4;
    for (int i = 0; i < channels->rowCount(); ++i)   // -- WP UFW -- as Modern names it (UXP A1)
        every = every && p.row(QStringLiteral("channel:%1").arg(channels->get(i, QStringLiteral("channel")).toInt())).value(QStringLiteral("title")) == ufwTitle(channels, i);
    p.check("every channel, by its name", every);
    p.check("no device: the open one is the only one", ids.filter(QRegularExpression(QStringLiteral("^device:"))).isEmpty());

    // What a row shows
    p.type(QStringLiteral("tube"));
    p.check("a panel's row says its page and its shortcut",
            p.currentId() == QLatin1String("tools.tube") && p.shownText(QStringLiteral("tools.tube"), "where").contains(QStringLiteral("Effects"))
                && p.shownText(QStringLiteral("tools.tube"), "keys") == QLatin1String("Ctrl+Shift+D"));
    p.type(QStringLiteral("line width"));
    p.check("a setting is found by its label, which shows",
            p.currentId() == QLatin1String("settings:graphing") && p.shownText(QStringLiteral("settings:graphing"), "where").contains(QStringLiteral("Line Width")));
    p.type(QStringLiteral("gjxq"));
    QQuickItem *none = p.d.item(QStringLiteral("CommandPalette.none"));
    p.check("nothing found is said", p.rows().isEmpty() && none && none->isVisible());
    p.key(Qt::Key_Return);
    p.check("and Enter does nothing then", p.opened());
    p.close();
}

// ---- Finding, as typed -------------------------------------------------------------

void findPart(Palette &p) {
    const std::pair<const char *, const char *> wanted[] = {
        { "matrix", "tools.matrix" }, { "mtrx", "tools.matrix" }, { "crossfeed", "tools.crossfeed" }, { "cfeed", "tools.crossfeed" },
        { "sg", "tools.signalGenerator" }, { "save to dev", "device.commit" }, { "commit", "device.commit" }, { "fact res", "device.factoryReset" },
        { "reset factory", "device.factoryReset" }, { "nerbs", "tools.stats" }, { "peq", "page:equaliser/peq" },
        { "effects", "section:effects" }, { "headphone prof", "autoeq.browse" }, { "firmware update", "device.firmwareUpdate" },
        { "ctrl+m", "device.mute" }, { "a/b", "device.abToggle" }, { "classic", "app.classicLayout" }, { "PSYCHO", "tools.psybass" },
        { "preset 2", "device.preset2" }, { "mute", "device.mute" }, { "ai agents", "settings:ai" }
    };
    p.open();
    for (const auto &[query, id] : wanted) {
        p.type(QLatin1String(query));
        const QString first = p.ids().value(0);
        // Toggle A/B is greyed without a compare: first, but not chosen
        p.check(QStringLiteral("\"%1\" finds %2 first (%3)").arg(QLatin1String(query), QLatin1String(id), first), first == QLatin1String(id));
    }
    p.close();
}

// ---- -- WP UFW -- (UXP A8) The words users type, and the manual's answers ---------------

void wordsPart(Palette &p) {
    const std::pair<const char *, const char *> wanted[] = {
        { "high-pass", "page:equaliser/crossover" }, { "hpf", "page:equaliser/crossover" }, { "protection", "settings:installer" },
        { "tweeter floor", "settings:installer" }, { "refused", "help:refused" }, { "rename device", "settings:global" },
        { "save to device", "device.commit" }, { "commit parameters", "device.commit" }, { "glossary", "help:words" },
        { "user manual", "help.manual" }
    };
    p.open();
    for (const auto &[query, id] : wanted) {
        p.type(QLatin1String(query));
        const QString first = p.ids().value(0);
        p.check(QStringLiteral("ufw: \"%1\" finds %2 first (%3)").arg(QLatin1String(query), QLatin1String(id), first), first == QLatin1String(id));
    }
    p.type(QStringLiteral("high-pass"));
    p.check("ufw: a synonym says what found it", p.shownText(QStringLiteral("page:equaliser/crossover"), "where").contains(QStringLiteral("high-pass")));
    p.type(QStringLiteral("save"));
    p.check("ufw: Ctrl+S reads Save to Device", p.row(QStringLiteral("device.commit")).value(QStringLiteral("title")).toString() == QStringLiteral("Save to Device…")
                && p.row(QStringLiteral("device.commit")).value(QStringLiteral("keys")).toString() == QLatin1String("Ctrl+S"));
    // The manual's answers open its page on the project's site, at the section
    p.type(QStringLiteral("help"));
    const QString refused = p.row(QStringLiteral("help:refused")).value(QStringLiteral("arg")).toString();
    p.check("ufw: Help entries open the manual's section", refused.startsWith(QStringLiteral("https://"))
                && refused.endsWith(QStringLiteral("/docs/manual/measurement.md#4-the-safety-rules-and-what-they-mean-for-you"))
                && p.row(QStringLiteral("help:refused")).value(QStringLiteral("kind")).toString() == QLatin1String("help"));
    p.check("ufw: the manual itself is a row the shell runs", p.shell->handles(QStringLiteral("help.manual")));
    // An output by its words (ChannelInfo's keywords, UXP A1): its number, its slot's name
    ChannelListModel *channels = p.d.app().device->channels();
    for (int i = 0; i < channels->rowCount(); ++i) {
        if (channels->get(i, QStringLiteral("kind")).toString() != QLatin1String("output")) continue;
        const QString id = QStringLiteral("channel:%1").arg(channels->get(i, QStringLiteral("channel")).toInt());
        const QString number = QString::number(channels->get(i, QStringLiteral("number")).toInt() + 1);
        p.type(QStringLiteral("out ") + number);
        p.check("ufw: an output by its number", p.ids().mid(0, 3).contains(id) && p.row(id).value(QStringLiteral("title")).toString() == ufwTitle(channels, i));
        const QString name = channels->get(i, QStringLiteral("displayName")).toString();
        if (!name.isEmpty() && name != channels->get(i, QStringLiteral("label")).toString()) {
            p.type(name);
            p.check("ufw: a layout's output by its name, its number second", p.ids().value(0) == id && p.row(id).value(QStringLiteral("title")).toString().startsWith(name + QStringLiteral(" · ")));
        }
        break;
    }
    p.close();
}

// ---- Choosing ------------------------------------------------------------------------

void runPart(Palette &p) {
    p.check("Enter on a panel's row shows its page", p.runTyped(QStringLiteral("tube"), QStringLiteral("tools.tube")) && p.at() == QLatin1String("effects/tube"));
    p.check("a page that is no panel", p.runTyped(QStringLiteral("delay gain"), QStringLiteral("page:equaliser/delay")) && p.at() == QLatin1String("equaliser/delay"));
    p.check("a section, where the person last was in it",
            p.runTyped(QStringLiteral("effects"), QStringLiteral("section:effects")) && p.at() == QLatin1String("effects/tube"));
    p.open();
    p.type(QStringLiteral("loudness"));
    p.check("a click on a row", p.click(p.rowItem(QStringLiteral("tools.loudness"))) && p.closes() && p.d.waitFor(1000, [&] { return p.at() == QLatin1String("effects/loudness"); }));
    p.settle();

    // The arrows, round the end
    p.open();
    p.type(QStringLiteral("preset"));
    const int n = int(p.rows().size());
    const int start = p.current();
    p.key(Qt::Key_Down);
    const int down = p.current();
    p.key(Qt::Key_Up);
    const int back = p.current();
    p.key(Qt::Key_Up);
    const int around = p.current();
    p.check("Down and Up move, round the end", n > 5 && start == 0 && down == 1 && back == 0 && around > n / 2);
    p.check("the current row shows", p.rowItem(p.currentId()) && p.rowItem(p.currentId())->isVisible());
    p.key(Qt::Key_PageUp);
    p.check("Page Up goes up", p.current() < around);
    p.key(Qt::Key_Tab);
    p.check("Tab stays in it", p.field()->hasActiveFocus());
    p.close();

    // A channel
    const int channel = p.d.app().device->channels()->get(p.d.app().device->channels()->rowCount() - 2, QStringLiteral("channel")).toInt();
    const QString label = p.d.app().device->channels()->get(p.d.app().device->channels()->rowCount() - 2, QStringLiteral("label")).toString();
    p.go("equaliser", "peq");
    p.go("overview", "overview");
    p.check("a channel opens on the Equaliser", p.runTyped(label, QStringLiteral("channel:%1").arg(channel)) && p.sc->selectedChannel() == channel
                && p.at() == QLatin1String("equaliser/peq"));
    p.sc->select(-1);

    // A settings page, on the page and (RB) in its own window
    p.check("a settings page opens on the Settings page",
            p.runTyped(QStringLiteral("unwrap"), QStringLiteral("settings:graphing")) && p.pageKey() == QLatin1String("settings/settings")
                && p.settings->currentPage() == QLatin1String("graphing"));
    p.go("overview", "overview");
    QWindow *window = p.shell->openWindow(QStringLiteral("edit.settings"));
    p.check("Settings in its own window", window && p.d.waitFor(2000, [&] { return window->isVisible(); }));
    p.settle();
    p.check("rb: the search shows it there, not on the hidden page",
            p.runTyped(QStringLiteral("peak hold"), QStringLiteral("settings:spectrum")) && p.settings->currentPage() == QLatin1String("spectrum")
                && p.at() == QLatin1String("overview/overview") && window && window->isVisible()
                && p.d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == window; }));
    p.shell->closeWindow(QStringLiteral("edit.settings"));
    p.settle();

    // A command on the device
    auto *keys = Registrar::instance<ShortcutController>();
    const bool muted = keys->muted();
    p.check("a command runs", p.runTyped(QStringLiteral("mute"), QStringLiteral("device.mute")) && p.d.waitFor(3000, [&] { return keys->muted() != muted; }));
    p.check("and again", p.runTyped(QStringLiteral("mute"), QStringLiteral("device.mute")) && p.d.waitFor(3000, [&] { return keys->muted() == muted; }));

    // A text field's own edits: the field that had the keyboard
    p.go("settings", "settings");
    QQuickItem *search = p.d.item(QStringLiteral("Settings.search"));
    if (search) {
        search->forceActiveFocus();
        search->setProperty("text", QStringLiteral("abc"));
        p.check("Select All is the field's that had the keyboard", p.runTyped(QStringLiteral("select all"), QStringLiteral("edit.selectAll"))
                    && search->property("selectedText").toString() == QLatin1String("abc"));
        p.check("and so is Delete", p.runTyped(QStringLiteral("delete"), QStringLiteral("edit.delete")) && search->property("text").toString().isEmpty());
        search->setProperty("text", QString());
        search->setFocus(false);
    }
    p.go("overview", "overview");
}

// ---- Greyed rows ------------------------------------------------------------------------

void greyedPart(Palette &p) {
    QSignalSpy triggered(p.shell, &Shell::triggered);
    p.open();
    p.type(QStringLiteral("save file"));
    const QVariantMap row = p.row(QStringLiteral("file.saveFile"));
    p.check("Save File is there, greyed (no file is open)", !row.isEmpty() && !row.value(QStringLiteral("enabled")).toBool());
    QQuickItem *item = p.rowItem(QStringLiteral("file.saveFile"));
    p.check("it is not the current row", p.currentId() != QLatin1String("file.saveFile") && item && !item->isEnabled());
    p.click(item);
    p.d.spin(100);
    p.check("a click on it does nothing", p.opened() && triggered.isEmpty());
    // An entry that greys after the list was made
    if (QObject *popup = p.popup()) {
        auto *history = Registrar::instance<HistoryController>();
        p.type(QStringLiteral("reconnect"));
        QVariant gone;
        QMetaObject::invokeMethod(popup, "choose", Q_RETURN_ARG(QVariant, gone), Q_ARG(QVariant, 999));
        p.check("a row that is not there is not run", p.opened() && history);
    }
    p.close();
    p.check("nothing ran", triggered.isEmpty());
}

// ---- Under it no shortcut works ------------------------------------------------------------

void keysPart(Palette &p) {
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *cc = Registrar::instance<ChannelController>();
    auto *keys = Registrar::instance<ShortcutController>();
    if (!snaps || !cc || !keys) return;
    QSignalSpy stored(snaps, &SnapshotsController::stored);
    const int had = int(snaps->snapshots().size());
    snaps->capture(QStringLiteral("Lay6"), QString());
    p.check("a snapshot to compare with", p.d.waitFor(5000, [&] { return snaps->snapshots().size() == had + 1 && !snaps->busy(); }));
    const QString id = snaps->snapshots().isEmpty() ? QString() : snaps->snapshots().first().toMap().value(QStringLiteral("id")).toString();
    cc->setOutputGain(0, -4);
    p.settle();
    snaps->abStart(QString(), id);
    p.check("the A/B compare runs", p.d.waitFor(8000, [&] { return snaps->abActive() && snaps->canToggle(); }) && p.d.waitIdle());
    const QString side = snaps->abSide();
    const bool muted = keys->muted();
    p.main->contentItem()->forceActiveFocus();

    QSignalSpy triggered(p.shell, &Shell::triggered);
    p.open();
    p.key(Qt::Key_B);
    p.check("a bare B types", p.field()->property("text").toString() == QLatin1String("b"));
    p.key(Qt::Key_M, Qt::ControlModifier);
    p.key(Qt::Key_M, Qt::ControlModifier | Qt::ShiftModifier);
    p.d.spin(200);
    p.check("no shortcut of the window works under it", triggered.isEmpty() && snaps->abSide() == side && keys->muted() == muted && p.opened());
    // With the keyboard on the list rather than the field
    if (QQuickItem *list = p.list()) list->forceActiveFocus();
    p.key(Qt::Key_B);
    p.d.spin(200);
    p.check("nor a bare B with the keyboard beside the field", triggered.isEmpty() && snaps->abSide() == side && p.opened());
    p.close();

    p.main->contentItem()->forceActiveFocus();
    p.key(Qt::Key_B);
    p.check("closed, a bare B toggles again", p.d.waitFor(8000, [&] { return snaps->abSide() != side; }) && p.d.waitIdle());
    p.d.waitFor(3000, [&] { return snaps->canToggle(); });
    // Also where a text field had the keyboard, whose key a bare B would be
    p.go("settings", "settings");
    QQuickItem *search = p.d.item(QStringLiteral("Settings.search"));
    if (search) search->forceActiveFocus();
    p.check("Toggle A/B, from a field", p.runTyped(QStringLiteral("toggle"), QStringLiteral("device.abToggle"))
                && p.d.waitFor(8000, [&] { return snaps->abSide() == side; }) && p.d.waitIdle());
    if (search) search->setFocus(false);
    snaps->abKeep(QStringLiteral("a"));
    p.d.waitFor(5000, [&] { return !snaps->abActive(); });
    snaps->remove(id);
    p.d.waitFor(5000, [&] { return snaps->snapshots().size() == had && !snaps->busy(); });
    cc->setOutputGain(0, 0);
    p.settle();
    p.go("overview", "overview");
}

// ---- Other windows, dialogs, the Interrupt Monitor --------------------------------------------

void windowsPart(Palette &p) {
    auto *mon = Registrar::instance<MonitorController>();
    QWindow *window = p.shell->openWindow(QStringLiteral("tools.interruptMonitor"));
    p.check("the Interrupt Monitor in its window", window && p.d.waitFor(2000, [&] { return window->isVisible(); }));
    if (!window || !mon) return;
    const auto line = [&] {
        const QString name = p.sc->slotNames().value(0) == QLatin1String("Lay6 A") ? QStringLiteral("Lay6 B") : QStringLiteral("Lay6 A");
        p.sc->renameChannel(0, name);
        return p.d.waitFor(4000, [&] { return mon->lines()->rowCount() > 0; });
    };
    const bool lines = line();
    p.settle();
    p.activate(window);
    const int had = mon->lines()->rowCount();
    QTest::keyClick(window, Qt::Key_K, Qt::ControlModifier);
    p.check("Ctrl+K in a tool window opens it in the main one", p.opens() && p.d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == p.main; }));
    p.check("and leaves the monitor's lines", lines && had > 0 && mon->lines()->rowCount() >= had);
    p.close();

    // A dialog asks something in the main window
    QObject *ask = p.d.object(QStringLiteral("GlobalControls.clearQuestion"));
    if (ask) QMetaObject::invokeMethod(ask, "open");
    p.check("a question is open", ask && p.d.waitFor(1500, [&] { return ask->property("opened").toBool(); }));
    p.ctrlK();
    p.d.spin(150);
    p.check("it does not open over a question", !p.opened());
    p.activate(window);
    QTest::keyClick(window, Qt::Key_K, Qt::ControlModifier);
    p.d.spin(200);
    p.check("nor from another window", !p.opened());
    if (ask) QMetaObject::invokeMethod(ask, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    p.d.waitFor(1500, [&] { return ask && !ask->property("visible").toBool(); });
    p.shell->closeWindow(QStringLiteral("tools.interruptMonitor"));
    p.settle();

    // On its page
    p.activate(p.main);
    p.go("analyse", "interruptMonitor");
    p.check("lines on the page", line());
    p.settle();
    const int onPage = mon->lines()->rowCount();
    p.ctrlK();
    p.check("Ctrl+K on the monitor's page is the palette's", p.opens() && mon->lines()->rowCount() >= onPage);
    p.key(Qt::Key_L, Qt::ControlModifier);
    p.d.spin(100);
    p.check("under it the monitor's Ctrl+L does not work", mon->lines()->rowCount() >= onPage);
    p.close();
    p.main->contentItem()->forceActiveFocus();
    p.key(Qt::Key_L, Qt::ControlModifier);
    p.check("Ctrl+L clears the monitor", mon->lines()->rowCount() == 0);
    p.sc->renameChannel(0, QString());
    p.settle();
    p.go("overview", "overview");
}

// ---- Markup in names, Getting Started, the sender ------------------------------------------------

void namesPart(Palette &p) {
    p.sc->renameChannel(0, QStringLiteral("<b>Rm</b>"));
    p.settle();
    p.open();
    p.type(QStringLiteral("<b>rm"));
    QQuickItem *title = p.d.item(QStringLiteral("CommandPalette.row.channel:0.title"));
    p.check("a channel's name is text, not markup", p.currentId() == QLatin1String("channel:0") && title && title->property("text") == QLatin1String("<b>Rm</b>")
                && title->property("textFormat").toInt() == 0);
    // Renamed under it
    p.close();
    p.sc->renameChannel(0, QString());
    p.settle();

    auto *ob = Registrar::instance<OnboardingController>();
    if (ob) {
        ob->requestSetup();
        p.d.spin(200);
        p.open();
        const QVariantMap effects = p.row(QStringLiteral("section:effects")), about = p.row(QStringLiteral("help.about"));
        p.check("under Getting Started the pages are greyed, the rest is not",
                ob->wizardShown() && !effects.value(QStringLiteral("enabled")).toBool() && about.value(QStringLiteral("enabled")).toBool());
        p.close();
        ob->finishSetup();
        p.d.spin(200);
    }

    auto *info = Registrar::instance<DeviceInfoController>();
    p.d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_generation(c, 8); });
    p.d.app().device->refresh();
    p.check("on the alink sender", info && p.d.waitFor(6000, [&] { return info->known() && info->sender(); }));
    p.settle();
    p.open();
    p.check("what it has no DSP for is greyed, what it keeps is not",
            !p.row(QStringLiteral("tools.matrix")).value(QStringLiteral("enabled")).toBool()
                && !p.row(QStringLiteral("section:effects")).value(QStringLiteral("enabled")).toBool()
                && !p.row(QStringLiteral("settings:inputs")).value(QStringLiteral("enabled")).toBool()
                && p.row(QStringLiteral("section:devices")).value(QStringLiteral("enabled")).toBool()
                && p.row(QStringLiteral("edit.settings")).value(QStringLiteral("enabled")).toBool()
                && p.row(QStringLiteral("settings:about")).value(QStringLiteral("enabled")).toBool());
    p.close();
    p.d.app().device->openDemo();
    p.check("back on the demo", p.d.waitFor(5000, [&] { return p.d.app().device->connected() && info->known() && !info->sender(); }));
    p.settle();
}

// ---- Several devices -----------------------------------------------------------------------------

void devicesPart(Palette &p) {
    auto *registry = DeviceRegistry::instance();
    const QString first = registry->activeId();
    registry->openDemo();
    p.check("a second device beside it", p.d.waitFor(15000, [&] { return registry->devices().size() == 2 && registry->activeId() != first; }));
    const QString second = registry->activeId();
    p.settle();
    p.check("Switch to the other device", p.runTyped(QStringLiteral("switch to"), QStringLiteral("device:") + first)
                && p.d.waitFor(3000, [&] { return registry->activeId() == first; }));
    p.settle();
    p.open();
    p.check("then the second is the other one, and comes first", p.currentId() == QStringLiteral("device:") + second
                && p.row(QStringLiteral("device:") + second).value(QStringLiteral("title")).toString().startsWith(QLatin1String("Switch to "))
                && p.row(QStringLiteral("device:") + first).isEmpty());
    // It closes under the palette
    p.key(Qt::Key_Down);
    const QString picked = p.currentId();
    registry->close(second);
    p.check("a device that closes leaves the list, the current row stays",
            p.d.waitFor(8000, [&] { return registry->devices().size() == 1 && p.row(QStringLiteral("device:") + second).isEmpty(); })
                && p.opened() && p.currentId() == picked);
    p.close();
    p.settle();
}

void galleryPart(Palette &p) {
    const Qt::ColorScheme before = p.shell->colorSchemeOverride();
    QDir(p.d.outDir()).mkpath(QStringLiteral("gallery"));
    for (const QSize &size : { QSize(1280, 800), QSize(1000, 660) }) {
        p.main->resize(size);
        p.settle();
        for (const bool dark : { true, false }) {
            p.shell->setColorSchemeOverride(dark ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
            for (const char *query : { "", "save", "line" }) {
                p.open();
                p.type(QLatin1String(query));
                p.d.spin(100);
                QQuickItem *list = p.list();
                QObject *popup = p.popup();
                p.check(QStringLiteral("it is inside the window at %1 px").arg(size.width()),
                        popup && list && popup->property("y").toReal() >= 0
                            && popup->property("y").toReal() + popup->property("height").toReal() <= p.main->height()
                            && popup->property("x").toReal() >= 0 && popup->property("width").toReal() <= p.main->width());
                const QString name = QStringLiteral("gallery/lay6-%1-%2-%3.png").arg(size.width())
                                         .arg(*query ? QLatin1String(query) : QLatin1String("all"), dark ? QLatin1String("dark") : QLatin1String("light"));
                p.check(QStringLiteral("screenshot %1").arg(name), p.d.grab(p.main, name));
                p.close();
            }
        }
    }
    p.shell->setColorSchemeOverride(before);
    p.main->resize(1280, 800);
    p.settle();
}

void paletteSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    Palette p{ d, d.app().shell, Registrar::instance<LayoutController>(), Registrar::instance<SettingsController>(),
               Registrar::instance<SidebarController>(), Registrar::instance<PaletteController>(), d.mainWindow() };
    p.check("the controllers", p.shell && p.layout && p.settings && p.sc && p.pc && p.main && DeviceRegistry::instance());
    if (!p.shell || !p.layout || !p.settings || !p.sc || !p.pc || !p.main) return;
    searchPart(p);
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    p.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    for (const QString &action : p.shell->shownWindows()) p.shell->closeWindow(action);
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->isVisible() && w != p.main && w->inherits("QQuickWindow")) w->close();
    const QSize was = p.main->size();
    const QString page = p.settings->currentPage();
    p.main->resize(1280, 800);
    p.sc->select(-1);
    p.activate(p.main);
    p.ctrlK();
    d.spin(100);
    p.check("Classic has none", !p.popup());
    p.check("Modern", switchTo(p, QStringLiteral("modern"), "ModernShell") && p.shell->pageHost());
    if (!p.shell->pageHost()) return;

    openPart(p);
    contentsPart(p);
    findPart(p);
    wordsPart(p);   // -- WP UFW --
    runPart(p);
    greyedPart(p);
    keysPart(p);
    windowsPart(p);
    namesPart(p);
    devicesPart(p);
    galleryPart(p);

    // To Classic by the palette itself
    p.check("Switch to the Classic Layout", p.runTyped(QStringLiteral("classic"), QStringLiteral("app.classicLayout"))
                && d.waitFor(3000, [&] { return !p.layout->modern(); }));
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    p.settle();
    p.check("in Classic again, without it", p.d.object(QStringLiteral("ClassicShell")) && !p.popup());
    p.settings->open(page);
    p.main->resize(was);
    d.spin(200);
}

const SmokeDriver::Section paletteSmoke("lay6-palette", 898, &paletteSection);

} // namespace
