// WP QD17: every Mac shortcut and its Ctrl key here, in one table, each pressed for
// real through QtGui's window-system key path (shortcut map first, then the focus
// item), as a user's key arrives. The Mac's list: DSPi_ConsoleApp.swift 11673-12046
// (menus), TestSignalsView.swift:941 (Space), InterruptMonitor.swift:1005-1010,
// PeqGraphEditor.swift:1316 (graph keys). Also an item carried from wave 2: the
// bridge counts the inputs the strips show (QD18's count). The other, a rescan
// keeping the demo, is in SmokeDriverDemo.cpp.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "HostAudioFormat.h"
#include "MonitorModel.h"
#include "PeqEditorItem.h"
#include "RtaController.h"
#include "SiggenController.h"
#include "TrayController.h"

#include <QClipboard>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSystemTrayIcon>
#include <cmath>
#include <memory>

// QtGui's entry for a key from the window system (QTest's, without linking QtTest)
Q_GUI_EXPORT void qt_handleKeyEvent(QWindow *w, QEvent::Type t, int k, Qt::KeyboardModifiers mods,
                                    const QString &text = QString(), bool autorep = false, ushort count = 1);

namespace {
constexpr Qt::KeyboardModifiers kCtrl = Qt::ControlModifier;
constexpr Qt::KeyboardModifiers kCtrlShift = Qt::ControlModifier | Qt::ShiftModifier;

void press(QWindow *w, int key, Qt::KeyboardModifiers mods, const QString &text = QString()) {
    qt_handleKeyEvent(w, QEvent::KeyPress, key, mods, text);
    qt_handleKeyEvent(w, QEvent::KeyRelease, key, mods, text);
}

QVariant inMain(QObject *root, const QString &expression) {
    QQmlExpression e(QQmlEngine::contextForObject(root), root, expression);
    return e.evaluate();
}

// A row of the Mac's menus that opens a window
struct WindowRow {
    const char *mac;
    const char *here;
    int key;
    Qt::KeyboardModifiers mods;
    const char *title;
};

// In the Mac's menu order; Settings and the AutoEQ browser last, as their search
// fields take the focus when they open
const WindowRow kWindowRows[] = {
    { "Cmd+Shift+M", "Ctrl+Shift+M", Qt::Key_M, kCtrlShift, "Matrix Mixer" },
    { "Cmd+Shift+L", "Ctrl+Shift+L", Qt::Key_L, kCtrlShift, "Loudness Compensation" },
    { "Cmd+Shift+X", "Ctrl+Shift+X", Qt::Key_X, kCtrlShift, "Crossfeed" },
    { "Cmd+Shift+P", "Ctrl+Shift+P", Qt::Key_P, kCtrlShift, "Psychoacoustic Bass" },
    { "Cmd+Shift+S", "Ctrl+Shift+S", Qt::Key_S, kCtrlShift, "Subharmonic Synthesizer" },
    { "Cmd+Shift+D", "Ctrl+Shift+D", Qt::Key_D, kCtrlShift, "Tube Modeller" },
    { "Cmd+Shift+U", "Ctrl+Shift+U", Qt::Key_U, kCtrlShift, "Stereo Upmixer" },
    { "Cmd+Shift+V", "Ctrl+Shift+V", Qt::Key_V, kCtrlShift, "Volume Leveller" },
    { "Cmd+Shift+G", "Ctrl+Shift+G", Qt::Key_G, kCtrlShift, "Signal Generator" },
    { "Cmd+Shift+A", "Ctrl+Shift+A", Qt::Key_A, kCtrlShift, "Spectrum Analyser" },
    { "Cmd+Shift+T", "Ctrl+Shift+T", Qt::Key_T, kCtrlShift, "System Statistics" },
    { "Cmd+Shift+I", "Ctrl+Shift+I", Qt::Key_I, kCtrlShift, "Interrupt Monitor" },
    { "Cmd+,", "Ctrl+,", Qt::Key_Comma, kCtrl, "Settings" },
    { "Cmd+Shift+B", "Ctrl+Shift+B", Qt::Key_B, kCtrlShift, "AutoEQ - Browse Profiles" },
};

struct Band { int type; float freq, gain, q; };

}

void SmokeDriver::testShortcuts() {
    section("QD17: every Mac shortcut has its Ctrl key");
    auto *b = m_.bridge;
    auto *main = qobject_cast<QQuickWindow *>(root());
    auto *tray = qobject_cast<TrayController *>(m_.engine->rootContext()->contextProperty(QStringLiteral("tray")).value<QObject *>());
    check("QD17: the main window, the tray", main && tray);
    check("QD17: on the demo device, writable", b->connected() && b->writable());
    if (!main || !tray || !b->writable()) return;
    const auto row = [this](const char *mac, const char *here, bool ok) {
        printf("smoke: QD17 %-12s -> %-13s %s\n", mac, here, ok ? "ok" : "FAILED");
        check(qPrintable(QStringLiteral("QD17: %1 is %2").arg(QLatin1String(mac), QLatin1String(here))), ok);
    };
    const auto toolWindows = [&]() {
        QList<QQuickWindow *> l;
        for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
            if (w != main && w->isVisible()) l.append(w);
        return l;
    };
    const auto hideTools = [&]() {
        for (QQuickWindow *w : toolWindows()) w->setVisible(false);
        spin(50);
    };
    const auto focusMain = [&]() {
        hideTools();
        main->requestActivate();
        return waitFor(1000, [&]() { return QGuiApplication::focusWindow() == main; });
    };
    // Whatever has the focus gets the key, as on a desktop
    const auto key = [&](int k, Qt::KeyboardModifiers mods, const QString &text = QString()) {
        QWindow *w = QGuiApplication::focusWindow();
        press(w ? w : main, k, mods, text);
        spin(100);
    };

    // The channel pages' bands as found: written back at the end
    const int channels[] = { 0, 1, 2, 3 };   // IN1/2 (maybe linked), Out 1/2
    std::vector<Band> saved;
    for (int ch : channels)
        for (int band = 0; band < BANDS_PER_CHANNEL; band++)
            saved.push_back({ b->filterType(ch, band), b->filterFreq(ch, band), b->filterGain(ch, band), b->filterQ(ch, band) });

    // A popup left open by an earlier pass (Qt 6.9's text context menu, from a
    // right-click) takes every key, as it would from a user: close it as a click would
    for (QObject *o : root()->findChildren<QObject *>())
        if (o->inherits("QQuickPopup") && o->property("visible").toBool()) {
            printf("smoke: QD17 closing a popup left open: %s\n", o->metaObject()->className());
            QMetaObject::invokeMethod(o, "close");
        }
    spin(100);

    // ── The windows, each from whichever window has the focus ──
    check("QD17: the main window takes the focus", focusMain());
    for (const WindowRow &r : kWindowRows) {
        const QList<QQuickWindow *> before = toolWindows();
        key(r.key, r.mods);
        QQuickWindow *opened = nullptr;
        waitFor(1000, [&]() {
            for (QQuickWindow *w : toolWindows())
                if (!before.contains(w)) opened = w;
            return opened != nullptr;
        });
        row(r.mac, r.here, opened && opened->title() == QLatin1String(r.title));
        if (opened) waitFor(1000, [&]() { return QGuiApplication::focusWindow() == opened; });
        // The Matrix's toggles, from its own window too
        if (opened && r.key == Qt::Key_M) {
            key(r.key, r.mods);
            check("QD17: Ctrl+Shift+M again closes the Matrix", !opened->isVisible());
            key(r.key, r.mods);
            check("QD17: and opens it again", waitFor(1000, [&]() { return opened->isVisible(); }));
        }
        // The Interrupt Monitor's own keys
        if (opened && r.key == Qt::Key_I) {
            waitFor(1000, [&]() { return opened->isActive(); });
            const bool paused = m_.monitor->paused();
            key(Qt::Key_P, kCtrl);
            const bool toggled = m_.monitor->paused() == !paused;
            key(Qt::Key_P, kCtrl);
            row("Cmd+P", "Ctrl+P", toggled && m_.monitor->paused() == paused);
            key(Qt::Key_K, kCtrl);
            row("Cmd+K", "Ctrl+K", m_.monitor->count() == 0);
        }
    }
    hideTools();

    // Off during Getting Started, as the Mac's menus are; Settings stays
    root()->setProperty("setupRequested", true);
    spin(100);
    check("QD17: the main window takes the focus during setup", focusMain());
    key(Qt::Key_L, kCtrlShift);
    check("QD17: setup: Ctrl+Shift+L opens nothing", toolWindows().isEmpty());
    key(Qt::Key_Comma, kCtrl);
    check("QD17: setup: Ctrl+, still opens Settings", waitFor(1000, [&]() { return toolWindows().size() == 1; }));
    root()->setProperty("setupRequested", false);
    spin(100);

    // ── The main window's own ──
    check("QD17: back on the main window", focusMain());
    key(Qt::Key_S, kCtrl);
    row("Cmd+S", "Ctrl+S", inMain(root(), QStringLiteral("commitDialog.visible")).toBool());
    inMain(root(), QStringLiteral("commitDialog.close()"));
    // -- WP QD39 -- The dialog is modal until its exit fade ends, and a modal
    // popup blocks the window's shortcuts: wait for it to hide, not a fixed time
    check("QD39: the commit dialog hides", waitFor(2000, [&]() { return !inMain(root(), QStringLiteral("commitDialog.visible")).toBool(); }));
    root()->setProperty("pendingFileAction", QString());
    key(Qt::Key_I, kCtrl);
    row("Cmd+I", "Ctrl+I", root()->property("pendingFileAction").toString() == QLatin1String("importFilters"));
    inMain(root(), QStringLiteral("openDialog.close()"));
    check("QD17: the main window again", focusMain());
    key(Qt::Key_E, kCtrl);
    row("Cmd+E", "Ctrl+E", root()->property("pendingFileAction").toString() == QLatin1String("exportFilters"));
    inMain(root(), QStringLiteral("saveDialog.close()"));
    root()->setProperty("pendingFileAction", QString());

    // Copy / Paste Parameters on the open channel page; text fields keep their own.
    // Two outputs (app ids 2 and 3): inputs may be a linked pair, which mirrors
    check("QD17: the main window for the clipboard", focusMain());
    QMetaObject::invokeMethod(root(), "selectOverview");
    b->setFilter(2, 2, 1, 2345.0f, 3.0f, 1.1f);
    b->setFilter(3, 2, 1, 1111.0f, 3.0f, 1.1f);
    QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 0));
    spin(300);
    key(Qt::Key_C, kCtrl);
    QQmlComponent fieldType(m_.engine);
    fieldType.setData("import QtQuick 2.15\nimport QtQuick.Controls 2.15\nTextField { width: 120; height: 30; text: \"QD17 text\" }",
                      QUrl(QStringLiteral("qrc:/qd17/Field.qml")));
    std::unique_ptr<QQuickItem> field(qobject_cast<QQuickItem *>(fieldType.create(QQmlEngine::contextForObject(root()))));
    check("QD17: a text field", field != nullptr);
    if (field) {
        field->setParentItem(main->contentItem());
        b->setFilter(2, 2, 1, 3456.0f, 3.0f, 1.1f);
        QGuiApplication::clipboard()->setText(QString());
        field->forceActiveFocus();
        spin(50);
        check("QD17: the field has the focus", root()->property("textFocused").toBool());
        key(Qt::Key_A, kCtrl);
        row("Cmd+A", "Ctrl+A (text)", field->property("selectedText").toString() == QLatin1String("QD17 text"));
        key(Qt::Key_C, kCtrl);
        row("Cmd+C", "Ctrl+C (text)", QGuiApplication::clipboard()->text() == QLatin1String("QD17 text"));
        key(Qt::Key_X, kCtrl);
        row("Cmd+X", "Ctrl+X (text)", field->property("text").toString().isEmpty() && QGuiApplication::clipboard()->text() == QLatin1String("QD17 text"));
        key(Qt::Key_V, kCtrl);
        row("Cmd+V", "Ctrl+V (text)", field->property("text").toString() == QLatin1String("QD17 text"));
        field->setVisible(false);
        field->setParentItem(nullptr);
        field.reset();
        main->contentItem()->forceActiveFocus();
        spin(50);
    }
    QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 1));
    spin(300);
    check("QD17: on Out 2's page, no text focused", root()->property("openChannelId").toInt() == 3 && !root()->property("textFocused").toBool());
    key(Qt::Key_V, kCtrl);
    const float pasted = b->filterFreq(3, 2);
    printf("smoke: QD17 pasted band 3 on Out 2: %.1f Hz\n", pasted);
    row("Cmd+C", "Ctrl+C (channel)", std::fabs(pasted - 2345.0f) < 0.5f);
    row("Cmd+V", "Ctrl+V (channel)", std::fabs(pasted - 2345.0f) < 0.5f);
    check("QD17: the text copy left the channel clipboard alone", std::fabs(pasted - 3456.0f) > 0.5f);

    // ── The graph's keys (PeqGraphEditor.swift:1316), on channel 0's page, two bands live ──
    b->setFilter(0, 2, 1, 1000.0f, 3.0f, 1.0f);
    b->setFilter(0, 3, 1, 500.0f, 2.0f, 1.0f);
    QMetaObject::invokeMethod(root(), "selectChannel", Q_ARG(QVariant, 0));
    spin(800);
    PeqEditorItem *editor = nullptr;
    for (PeqEditorItem *e : root()->findChildren<PeqEditorItem *>())
        if (e->active() && e->window() == main) editor = e;
    check("QD17: the channel page's graph editor", editor != nullptr);
    if (editor) {
        editor->forceActiveFocus();
        spin(50);
        key(Qt::Key_A, kCtrl);
        row("Cmd+A", "Ctrl+A (graph)", editor->selectedBands().size() >= 2);
        key(Qt::Key_Tab, Qt::NoModifier);
        const QVariantList one = editor->selectedBands();
        row("Tab", "Tab (graph)", one.size() == 1);
        const int band = one.isEmpty() ? 2 : one.first().toInt();
        const float freq = b->filterFreq(0, band), gain = b->filterGain(0, band), q = b->filterQ(0, band);
        key(Qt::Key_Right, Qt::NoModifier);
        row("Right", "Right (graph)", waitFor(1000, [&]() { return b->filterFreq(0, band) > freq * 1.03f; }));
        key(Qt::Key_Up, Qt::NoModifier);
        row("Up", "Up (graph)", waitFor(1000, [&]() { return b->filterGain(0, band) != gain || b->filterQ(0, band) != q; }));
        // -- WP QD36 -- Under Qt 6.11 the hidden windows' Escape shortcuts matched in the
        // main window too, ambiguously, and Escape never reached the graph; they now
        // match only while their window is active
        key(Qt::Key_Escape, Qt::NoModifier);
        row("Escape", "Escape (graph)", editor->selectedBands().isEmpty());
        editor->deselectAll();
    }
    // -- WP QD36 -- With the focus, Settings still closes on its own Escape
    QQuickWindow *settingsWin = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settingsWin = w;
    if (settingsWin) {
        settingsWin->setVisible(true);
        settingsWin->requestActivate();
        const bool focused = waitFor(1000, [&]() { return QGuiApplication::focusWindow() == settingsWin; });
        spin(50);
        key(Qt::Key_Escape, Qt::NoModifier);
        check("QD36: Escape closes Settings while it has the focus", focused && waitFor(1000, [&]() { return !settingsWin->isVisible(); }));
        settingsWin->setVisible(false);
        focusMain();
    } else {
        check("QD36: the Settings window", false);
    }
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(200);

    // ── Space in the Signal Generator ──
    QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("siggen")));
    QQuickWindow *siggenWin = root()->findChild<QQuickWindow *>(QStringLiteral("siggenWindow"));
    check("QD17: the Signal Generator, active", siggenWin && waitFor(1000, [&]() { return siggenWin->isActive(); }));
    if (siggenWin) {
        if (!siggenWin->property("blocker").toString().isEmpty()) m_.siggen->selectAllOutputs();
        spin(50);
        printf("smoke: QD17 siggen blocker: '%s'\n", qPrintable(siggenWin->property("blocker").toString()));
        key(Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
        const bool started = waitFor(2000, [&]() { return m_.siggen->running(); });
        key(Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
        const bool stopped = waitFor(3000, [&]() { return !m_.siggen->running(); });
        row("Space", "Space (generator)", started && stopped);
        if (m_.siggen->running()) m_.siggen->stopNow();
        siggenWin->setVisible(false);
    }

    // ── Quit asks about unsaved changes, as closing does ──
    b->setFilter(1, 2, 1, 4567.0f, 2.0f, 1.0f);
    check("QD17: changes unsaved, so Quit asks", waitFor(1000, [&]() { return b->presetDirty(); }));
    if (b->presetDirty()) {
        check("QD17: the main window for Quit", focusMain());
        key(Qt::Key_Q, kCtrl);
        row("Cmd+Q", "Ctrl+Q", inMain(root(), QStringLiteral("unsavedDialog.visible")).toBool() && main->isVisible());
        inMain(root(), QStringLiteral("unsavedDialog.chosen('cancel'); unsavedDialog.close()"));
        spin(300);
    }

    // ── Minimise to Menu Bar: the tray here ──
    check("QD17: the main window for the tray", focusMain());
    key(Qt::Key_M, Qt::ControlModifier | Qt::AltModifier);
    check("QD17: no tray: Ctrl+Alt+M leaves the window", main->isVisible() && !tray->windowHidden());
    tray->setAvailabilityProbe([]() { return true; });
    focusMain();
    key(Qt::Key_M, Qt::ControlModifier | Qt::AltModifier);
    row("Cmd+Opt+M", "Ctrl+Alt+M", tray->windowHidden() && !main->isVisible());
    tray->restore();
    check("QD17: the window comes back", waitFor(1000, [&]() { return main->isVisible() && !tray->windowHidden(); }));
    // As found: off, and offscreen's own answer
    tray->setShowInTray(false);
    tray->setAvailabilityProbe([]() { return QSystemTrayIcon::isSystemTrayAvailable(); });

    // The bands as found
    int at = 0;
    for (int ch : channels)
        for (int band = 0; band < BANDS_PER_CHANNEL; band++, at++)
            b->setFilter(ch, band, saved[at].type, saved[at].freq, saved[at].gain, saved[at].q);
    spin(200);
}

// The bridge's live input count is what the strips show: on the demo device, whose
// report says two, a host with eight holds all four pairs open
void SmokeDriver::testShownInputCount() {
    section("QD17: pairs, analyser and files count the shown inputs");
    auto *b = m_.bridge;
    auto *host = qobject_cast<HostAudioFormatController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("hostAudio")).value<QObject *>());
    check("QD17: hostAudio", host != nullptr);
    if (!host || !b->connected()) return;
    const int report = qMin(b->numInputChannels(), qMax(2, b->activeInputChannels()));
    check("QD17: bound from main.qml", b->shownInputCount() == host->inputCount() && b->liveInputCount() == report);
    check("QD17: the demo's report is one pair", report == 2 && !b->pairAvailable(b->inputAppId(2)));

    // A backend that says eight for any DSPi, used on the demo device
    class Eight : public HostAudio::Backend {
    public:
        std::vector<HostAudio::Endpoint> read() override {
            HostAudio::Endpoint e;
            e.text = QStringLiteral("Weeb Labs DSPi");
            e.channels = 8;
            return { e };
        }
        void startWatching(std::function<void()>) override {}
        void stopWatching() override {}
    };
    int statusSignals = 0;
    const QMetaObject::Connection counting = connect(b, &DSPiBridge::statusChanged, this, [&statusSignals]() { statusSignals++; });
    host->setBackendForTest(std::make_unique<Eight>(), true);
    check("QD17: the host says eight", waitFor(3000, [&]() { return host->inputCount() == 8; }));
    printf("smoke: QD17 shown %d, live %d, active %d\n", host->inputCount(), b->liveInputCount(), b->activeInputChannels());
    check("QD17: liveInputCount is the strips' count", b->liveInputCount() == 8 && b->activeInputChannels() == report);
    bool pairs = true;
    for (int in = 0; in < 8; in++) pairs = pairs && b->pairAvailable(b->inputAppId(in));
    check("QD17: every pair can link while the stream idles", pairs);
    check("QD17: the views heard of it", statusSignals > 0);
    check("QD17: the analyser offers the eight inputs", !m_.rta->supported() || m_.rta->availableChannels(0).size() == 8);
    disconnect(counting);

    // Back to the device's report
    host->setBackendForTest(HostAudio::makeBackend(), false);
    check("QD17: back to the report", waitFor(3000, [&]() { return host->inputCount() == report; }) && b->liveInputCount() == report
                                          && !b->pairAvailable(b->inputAppId(2)));
}

// -- WP QD22 -- Accessibility. Each main window's accessible tree is walked (QAccessible's
// own interfaces, as a screen reader gets them), and every control in it, a Quick
// Controls one or a custom one (a MouseArea someone clicks, a C++ item taking the
// mouse), must reach a node with a control's role and a name. Then Tab alone walks
// the window: every control takes the focus on the way, and (Qt 6, whose software
// backend grabs offscreen) the window looks different round it while it has it.
#include <QAccessible>
#include <QMetaMethod>
#include <QQmlContext>
#include <QHash>
#include <QImage>
#include <QSet>

namespace {
// QObject::isSignalConnected is protected; a member pointer through a subclass reads it
struct SignalPeek : QObject {
    using QObject::isSignalConnected;
};

// The Accessible attached object QML gave an item, if any (its parent is the item)
QObject *accessibleAttached(QObject *o) {
    for (QObject *c : o->children())
        if (c->inherits("QQuickAccessibleAttached")) return c;
    return nullptr;
}

bool isQuickControl(const QQuickItem *it) {
    return it->inherits("QQuickAbstractButton") || it->inherits("QQuickSlider") || it->inherits("QQuickComboBox")
        || it->inherits("QQuickSpinBox") || it->inherits("QQuickRangeSlider") || it->inherits("QQuickDial")
        || (it->inherits("QQuickTextInput") && !it->property("readOnly").toBool())
        || (it->inherits("QQuickTextEdit") && !it->property("readOnly").toBool());
}

// Something a user clicks: a Quick control, a MouseArea with a click handler or a
// drag, or a C++ item taking the mouse (the graph). Hover-only areas, Flickables
// and the Controls' own containers are not.
bool isInteractive(QQuickItem *it) {
    if (isQuickControl(it)) return true;
    if (it->inherits("QQuickMouseArea")) {
        // A right-click alone is a second way to a menu or a reset, not a control
        if (!(it->acceptedMouseButtons() & Qt::LeftButton)) return false;
        const QMetaObject *mo = it->metaObject();
        static const QSet<QByteArray> kSignals = { "clicked", "pressed", "doubleClicked", "pressAndHold", "released" };
        for (int i = 0; i < mo->methodCount(); i++) {
            const QMetaMethod m = mo->method(i);
            if (m.methodType() == QMetaMethod::Signal && kSignals.contains(m.name())
                && (it->*(&SignalPeek::isSignalConnected))(m))
                return true;
        }
        QObject *drag = it->property("drag").value<QObject *>();
        return drag && drag->property("target").value<QObject *>();
    }
    // Text takes the mouse for its links only
    if (it->inherits("QQuickText")) {
        const QMetaObject *mo = it->metaObject();
        const int i = mo->indexOfSignal("linkActivated(QString)");
        return i >= 0 && (it->*(&SignalPeek::isSignalConnected))(mo->method(i));
    }
    if (it->inherits("QQuickFlickable") || it->inherits("QQuickControl") || it->inherits("QQuickPopupItem")
        || it->inherits("QQuickTextInput") || it->inherits("QQuickTextEdit") || it->inherits("QQuickOverlay"))
        return false;
    // A C++ item of the app's own (the graph editor); Qt Quick's own items take
    // the mouse for a pointer handler (a row's HoverHandler) as well
    const QMetaObject *base = it->metaObject();
    while (base && QByteArray(base->className()).contains("_QML")) base = base->superClass();
    if (base && QByteArray(base->className()).startsWith("QQuick")) return false;
    return it->acceptedMouseButtons() != Qt::NoButton;
}

bool isControlRole(QAccessible::Role r) {
    switch (r) {
    case QAccessible::Button: case QAccessible::CheckBox: case QAccessible::RadioButton: case QAccessible::ComboBox:
    case QAccessible::Slider: case QAccessible::SpinBox: case QAccessible::EditableText: case QAccessible::PageTab:
    case QAccessible::ListItem: case QAccessible::MenuItem: case QAccessible::Link: case QAccessible::Cell:
    case QAccessible::ButtonMenu: case QAccessible::ButtonDropDown: case QAccessible::Dial: case QAccessible::Chart:
        return true;
    default:
        return false;
    }
}

// Where a failure is: the item's QML file and its nearest few QML types
QString describe(QQuickItem *it) {
    QStringList chain;
    for (QQuickItem *x = it; x && chain.size() < 5; x = x->parentItem()) {
        QString n = QString::fromLatin1(x->metaObject()->className());
        n = n.section(QLatin1String("_QML"), 0, 0);
        if (!x->objectName().isEmpty()) n += QLatin1Char('#') + x->objectName();
        chain << n;
    }
    QQmlContext *ctx = QQmlEngine::contextForObject(it);
    return (ctx ? ctx->baseUrl().fileName() : QStringLiteral("?")) + QLatin1String(": ") + chain.join(QLatin1String(" < "));
}

void collectAccessible(QAccessibleInterface *iface, QSet<QObject *> *seen, int depth) {
    if (!iface || depth > 200) return;
    if (QObject *o = iface->object()) {
        if (seen->contains(o)) return;
        seen->insert(o);
    }
    for (int i = 0; i < iface->childCount(); i++) collectAccessible(iface->child(i), seen, depth + 1);
}

void collectInteractive(QQuickItem *it, QList<QQuickItem *> *out) {
    // A MouseArea's own `enabled` is not the item's
    if (!it->isVisible() || !it->isEnabled() || !it->property("enabled").toBool() || it->opacity() <= 0.0) return;
    if (isInteractive(it)) out->append(it);
    for (QQuickItem *c : it->childItems()) collectInteractive(c, out);
}

bool within(QQuickItem *it, QQuickItem *scope) {
    for (QQuickItem *x = it; x; x = x->parentItem())
        if (x == scope) return true;
    return false;
}

// The visible item a screen reader knows by this name
QQuickItem *byAccessibleName(QQuickItem *it, const QString &name) {
    if (!it->isVisible()) return nullptr;
    if (QObject *att = accessibleAttached(it))
        if (att->property("name").toString() == name) return it;
    for (QQuickItem *c : it->childItems())
        if (QQuickItem *found = byAccessibleName(c, name)) return found;
    return nullptr;
}

// The screen reader's press on an item's node
bool accessiblePress(QQuickItem *it) {
    QAccessibleInterface *iface = QAccessible::queryAccessibleInterface(it);
    QAccessibleActionInterface *actions = iface ? iface->actionInterface() : nullptr;
    if (!actions || !actions->actionNames().contains(QAccessibleActionInterface::pressAction())) return false;
    actions->doAction(QAccessibleActionInterface::pressAction());
    return true;
}
}

// One window, or one popup in it (`scope`): the accessible tree, then a Tab walk
void SmokeDriver::a11yAudit(QQuickWindow *w, QQuickItem *scope, const QString &where, bool tabWalk) {
    QAccessible::setActive(true);   // the Controls attach their roles now, as for a screen reader
    spin(100);
    if (!scope) scope = w->contentItem();
    QSet<QObject *> tree;
    collectAccessible(QAccessible::queryAccessibleInterface(w), &tree, 0);
    QList<QQuickItem *> clicked;
    collectInteractive(scope, &clicked);
    // Each control's node: itself when it is one, else the nearest item above it
    // that QML made accessible. An area marked Accessible.ignored is decoration.
    QList<QQuickItem *> controls;
    int bad = 0;
    for (QQuickItem *p : clicked) {
        QObject *own = accessibleAttached(p);
        if (own && own->property("ignored").toBool()) continue;
        QQuickItem *node = nullptr;
        for (QQuickItem *x = p; x && !node; x = x->parentItem()) {
            QObject *att = accessibleAttached(x);
            if (isQuickControl(x) || (att && !att->property("ignored").toBool())) node = x;
        }
        QAccessibleInterface *iface = node ? QAccessible::queryAccessibleInterface(node) : nullptr;
        const QString name = iface ? iface->text(QAccessible::Name).trimmed() : QString();
        QString why;
        if (!iface) why = QStringLiteral("no accessible node");
        else if (!isControlRole(iface->role())) why = QStringLiteral("role %1").arg(int(iface->role()));
        else if (name.isEmpty()) why = QStringLiteral("no name");
        else if (!tree.contains(node)) why = QStringLiteral("not in the window's accessible tree");
        if (!why.isEmpty()) {
            bad++;
            printf("smoke: QD22 %s: %s: %s\n", qPrintable(where), qPrintable(why),
                   qPrintable(describe(node && isControlRole(iface->role()) ? node : p)));
            continue;
        }
        if (!controls.contains(node)) controls.append(node);
    }
    printf("smoke: QD22 %s: %d controls, %d without a role or name\n", qPrintable(where), int(controls.size()), bad);
    check(qPrintable(QStringLiteral("QD22: %1: every control has a role and a name").arg(where)), bad == 0 && !controls.isEmpty());
    if (!tabWalk) return;

    // Tab alone, from the first item, until the focus comes round again
    w->requestActivate();
    waitFor(1000, [&]() { return QGuiApplication::focusWindow() == w; });
    QQuickItem *start = w->activeFocusItem();
    if (!start || !within(start, scope)) {
        scope->forceActiveFocus(Qt::TabFocusReason);
        spin(20);
    }
    QList<QQuickItem *> order;
    QSet<QQuickItem *> reached;
    QHash<QQuickItem *, bool> shows;   // a control, and whether its focus showed
    QHash<QQuickItem *, QRect> shownAt;
    QSet<QQuickItem *> viaList;   // a list's rows: its current row shows, not a ring
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    const bool grabs = true;
#else
    const bool grabs = false;   // Qt 5's software backend grabs nothing offscreen
#endif
    QQuickItem *prev = nullptr;
    int same = 0;
    bool trapped = false;
    for (int step = 0; step < 600; step++) {
        press(w, Qt::Key_Tab, Qt::NoModifier);
        spin(15);
        QQuickItem *f = w->activeFocusItem();
        QQuickItem *owner = nullptr, *list = nullptr;
        // A list's rows: the list keeps the keys (Up/Down), so reaching it, or
        // its current row, reaches them
        for (QQuickItem *x = f; x && !list; x = x->parentItem())
            if (x->inherits("QQuickItemView")) list = x;
        for (QQuickItem *c : controls) {
            if (f && within(f, c) && (!owner || within(c, owner))) owner = c;
            if (list && within(c, list)) { reached.insert(c); viaList.insert(c); }
        }
        if (owner) reached.insert(owner);
        // The window round the control, with the focus and with the focus
        // taken off it and given back: the same scroll, the same everything else
        if (owner && grabs && !shows.value(owner)) {
            const QRect at = owner->mapRectToScene(QRectF(0, 0, owner->width(), owner->height())).toAlignedRect();
            shownAt[owner] = at;
            const QImage with = w->grabWindow();
            f->setFocus(false);
            spin(5);
            const QImage without = w->grabWindow();
            f->forceActiveFocus(Qt::TabFocusReason);
            spin(5);
            const qreal dpr = with.devicePixelRatio();
            const QRect r = at.adjusted(-4, -4, 4, 4).intersected(QRect(QPoint(), with.size() / dpr));
            const QRect px(QPoint(int(r.x() * dpr), int(r.y() * dpr)), QSize(int(r.width() * dpr), int(r.height() * dpr)));
            shows[owner] = !px.isEmpty() && with.copy(px) != without.copy(px);
        }
        if (!f) break;
        // An item may keep the focus for a few presses (the graph steps through
        // its bands), never for good
        if (f == prev && ++same > 2 * BANDS_PER_CHANNEL) { trapped = true; break; }
        if (f != prev) same = 0;
        if (f != prev && !order.isEmpty() && f == order.first()) break;   // round again
        if (!order.contains(f)) order.append(f);
        prev = f;
    }
    if (trapped) printf("smoke: QD22 %s: Tab is held by %s\n", qPrintable(where), qPrintable(describe(prev)));
    int missed = 0, unseen = 0;
    for (QQuickItem *c : controls) missed += reached.contains(c) ? 0 : 1;
    if (missed)
        for (int i = 0; i < order.size(); i++)
            printf("smoke: QD22 %s: Tab stop %d: %s\n", qPrintable(where), i + 1, qPrintable(describe(order[i])));
    missed = 0;
    for (QQuickItem *c : controls) {
        if (!reached.contains(c)) {
            missed++;
            printf("smoke: QD22 %s: Tab never reaches %s\n", qPrintable(where), qPrintable(describe(c)));
        } else if (grabs && !shows.value(c) && !viaList.contains(c)) {
            unseen++;
            const QRect r = shownAt.value(c);
            printf("smoke: QD22 %s: no visible focus on %s (at %d,%d %dx%d in %dx%d)\n", qPrintable(where), qPrintable(describe(c)),
                   r.x(), r.y(), r.width(), r.height(), w->width(), w->height());
        }
    }
    printf("smoke: QD22 %s: Tab stops %d, reached %d of %d controls\n", qPrintable(where), int(order.size()),
           int(reached.size()), int(controls.size()));
    check(qPrintable(QStringLiteral("QD22: %1: Tab reaches every control").arg(where)), missed == 0 && !trapped);
    check(qPrintable(QStringLiteral("QD22: %1: the focus shows on each").arg(where)), unseen == 0);
}

void SmokeDriver::testAccessibility() {
    section("QD22: accessible names and roles, a keyboard-only walk");
    auto *main = qobject_cast<QQuickWindow *>(root());
    check("QD22: the main window", main != nullptr);
    if (!main) return;
    const auto hideTools = [&]() {
        for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
            if (w != main && w->isVisible()) w->setVisible(false);
        spin(50);
    };
    hideTools();
    main->show();
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(300);
    a11yAudit(main, nullptr, QStringLiteral("main window"), true);
    QMetaObject::invokeMethod(root(), "selectChannel", Q_ARG(QVariant, 0));
    spin(600);
    a11yAudit(main, nullptr, QStringLiteral("main window, an input's EQ"), true);
    QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 0));
    spin(600);
    a11yAudit(main, nullptr, QStringLiteral("main window, an output's EQ"), true);
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(300);

    // Devices: a popup in the main window
    QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("devices")));
    spin(400);
    QObject *devices = root()->findChild<QObject *>(QStringLiteral("devicesPopup"));
    QQuickItem *devicesItem = devices ? devices->property("contentItem").value<QQuickItem *>() : nullptr;
    check("QD22: the devices list", devicesItem && devices->property("visible").toBool());
    if (devicesItem) a11yAudit(main, devicesItem->parentItem(), QStringLiteral("devices"), true);
    if (devices) QMetaObject::invokeMethod(devices, "close");
    spin(200);

    // Settings, every page
    QQuickWindow *settings = qobject_cast<QQuickWindow *>(inMain(root(), QStringLiteral("settingsWindow")).value<QObject *>());
    check("QD22: the Settings window", settings != nullptr);
    if (settings) {
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("settings")));
        spin(400);
        const QVariantList groups = settings->property("groups").toList();
        for (const QVariant &g : groups)
            for (const QVariant &p : g.toMap().value(QStringLiteral("pages")).toList()) {
                const QString id = p.toMap().value(QStringLiteral("id")).toString();
                QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, id));
                spin(400);
                if (settings->property("currentPage").toMap().value(QStringLiteral("id")).toString() != id) continue;   // not on this device
                a11yAudit(settings, nullptr, QStringLiteral("settings: %1").arg(id), true);
            }
        settings->setVisible(false);
        spin(100);
    }

    // Activation: Space and Return work a custom control as a click does, and so
    // does a screen reader's press. Each one is left as found.
    const auto focusOn = [&](QQuickWindow *w, QQuickItem *it) {
        w->requestActivate();
        waitFor(1000, [&]() { return QGuiApplication::focusWindow() == w; });
        it->forceActiveFocus(Qt::TabFocusReason);
        spin(30);
        return it->hasActiveFocus();
    };
    const auto keyOn = [&](QQuickWindow *w, int k, const QString &text) {
        press(w, k, Qt::NoModifier, text);
        spin(80);
    };
    hideTools();
    main->requestActivate();
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(200);
    auto *b = m_.bridge;
    if (QQuickItem *bypass = byAccessibleName(main->contentItem(), QStringLiteral("Bypass Master EQ"))) {
        const bool was = b->bypass();
        check("QD22: the bypass button takes the focus", focusOn(main, bypass));
        keyOn(main, Qt::Key_Space, QStringLiteral(" "));
        const bool spaced = waitFor(1000, [&]() { return b->bypass() != was; });
        keyOn(main, Qt::Key_Return, QString());
        const bool returned = waitFor(1000, [&]() { return b->bypass() == was; });
        check("QD22: Space and Return press the bypass button", spaced && returned);
        check("QD22: a screen reader presses it", accessiblePress(bypass) && waitFor(1000, [&]() { return b->bypass() != was; }));
        if (b->bypass() != was) b->setBypass(was);
    } else {
        check("QD22: the bypass button", false);
    }
    if (QQuickItem *preset = byAccessibleName(main->contentItem(), QStringLiteral("Preset"))) {
        check("QD22: the preset picker takes the focus", focusOn(main, preset));
        keyOn(main, Qt::Key_Space, QStringLiteral(" "));
        QObject *menu = nullptr;
        for (QObject *o : root()->findChildren<QObject *>())
            if (o->inherits("QQuickPopup") && o->property("visible").toBool()) menu = o;
        check("QD22: Space opens the preset menu", menu != nullptr);
        keyOn(main, Qt::Key_Escape, QString());
        check("QD22: Escape closes it", menu && waitFor(1000, [&]() { return !menu->property("visible").toBool(); }));
    } else {
        check("QD22: the preset picker", false);
    }
    if (settings) {
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("settings")));
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("about")));
        spin(300);
        const auto page = [&]() { return settings->property("currentPage").toMap().value(QStringLiteral("id")).toString(); };
        QQuickItem *graphing = byAccessibleName(settings->contentItem(), QStringLiteral("Graphing"));
        QQuickItem *advanced = byAccessibleName(settings->contentItem(), QStringLiteral("Advanced"));
        check("QD22: the Settings sidebar's pages", graphing && advanced);
        if (graphing && advanced) {
            check("QD22: a sidebar page takes the focus", focusOn(settings, graphing));
            keyOn(settings, Qt::Key_Space, QStringLiteral(" "));
            check("QD22: Space opens its page", waitFor(1000, [&]() { return page() == QLatin1String("graphing"); }));
            check("QD22: a screen reader opens another", accessiblePress(advanced) && waitFor(1000, [&]() { return page() == QLatin1String("advanced"); }));
        }
        settings->setVisible(false);
        spin(100);
    }
    QQuickWindow *matrix = root()->findChild<QQuickWindow *>(QStringLiteral("matrixWindow"));
    if (matrix) {
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("matrix")));
        spin(400);
        const QString in = b->channelName(b->inputAppId(0)), out = b->channelName(2);
        QQuickItem *point = byAccessibleName(matrix->contentItem(), in + QLatin1String(" to ") + out);
        check("QD22: the Matrix's first point", point != nullptr);
        if (point) {
            const bool was = b->matrixRouting(0, 0);
            check("QD22: it takes the focus", focusOn(matrix, point));
            keyOn(matrix, Qt::Key_Space, QStringLiteral(" "));
            const bool spaced = waitFor(1000, [&]() { return b->matrixRouting(0, 0) != was; });
            keyOn(matrix, Qt::Key_Return, QString());
            check("QD22: Space and Return switch a route", spaced && waitFor(1000, [&]() { return b->matrixRouting(0, 0) == was; }));
        }
        matrix->setVisible(false);
        spin(100);
    }

    // The tool windows
    for (const char *n : { "matrix", "graph", "loudness", "crossfeed", "leveller", "psybass", "subharm", "tube", "upmix",
                           "stats", "spectrum", "siggen", "monitor", "firmware", "whatsNew", "autoeq", "autoeqRebuild" }) {
        hideTools();
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QString::fromLatin1(n)));
        spin(500);
        QQuickWindow *found = nullptr;
        // -- WP QD39 -- A window that does not open fails here, not skipped; a
        // loaded host gets a little longer than the settle time to show it
        waitFor(2000, [&]() {
            for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
                if (w != main && w->isVisible()) found = w;
            return found != nullptr;
        });
        check(qPrintable(QStringLiteral("QD39: tool window %1 opens for the audit").arg(QString::fromLatin1(n))), found != nullptr);
        if (!found) continue;
        a11yAudit(found, nullptr, QString::fromLatin1(n), true);
    }
    hideTools();
    main->requestActivate();
    spin(100);
}
