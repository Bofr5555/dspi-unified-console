// WP QD41: a slider changed from the keyboard or by an accessibility action
// commits, as a drag's release does: once per change (on the key's release,
// not an auto-repeat; 400 ms after the last accessibility step), with the
// same rounding (the delay to whole ms, SL1). Before, Left/Right committed
// on every auto-repeat, Up/Down/PageUp/PageDown/Home/End did nothing, and an
// accessibility increase moved the thumb without sending anything.
#include "SmokeDriver.h"

#include "DSPiBridge.h"

#include <QAccessible>
#include <QCoreApplication>
#include <QKeyEvent>
#include <QMetaMethod>
#include <QQuickItem>
#include <QQuickWindow>
#include <algorithm>
#include <cmath>
#include <cstdio>

namespace {
// Counts a QML signal (LevelSection.committed / StyledSlider.settled)
class SignalCounter : public QObject {
    Q_OBJECT
public:
    SignalCounter(QObject *sender, const char *signature) : QObject(sender) {
        const int index = sender->metaObject()->indexOfSignal(signature);
        if (index >= 0)
            ok = bool(connect(sender, sender->metaObject()->method(index), this, metaObject()->method(metaObject()->indexOfSlot("hit()"))));
    }
    bool ok = false;
    int count = 0;
public slots:
    void hit() { count++; }
};

QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

void key(QQuickWindow *w, QEvent::Type type, int k, bool autoRepeat = false) {
    QKeyEvent e(type, k, Qt::NoModifier, QString(), autoRepeat);
    QCoreApplication::sendEvent(w, &e);
}

void tap(QQuickWindow *w, int k) {
    key(w, QEvent::KeyPress, k);
    key(w, QEvent::KeyRelease, k);
}

bool near(double a, double b) { return std::fabs(a - b) < 1e-3; }
bool whole(double ms) { return std::fabs(ms - std::round(ms)) < 1e-6; }
}

void SmokeDriver::testSliderKeys() {
    section("sliders commit from the keyboard and accessibility (QD41)");
    auto *b = m_.bridge;
    check("QD41: the demo device is writable", waitFor(2000, [b] { return b->connected() && b->writable(); }));
    auto *main = qobject_cast<QQuickWindow *>(root());
    check("QD41: the main window", main != nullptr);
    if (!main) return;
    main->show();
    main->requestActivate();
    const float gainBefore = b->outputGainDB(0), delayBefore = b->outputDelayMS(0);
    QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 0));
    spin(300);
    QQuickItem *page = nullptr;
    for (QQuickItem *i : root()->findChildren<QQuickItem *>())
        if (QString::fromLatin1(i->metaObject()->className()).startsWith(QLatin1String("ChannelSettingsView")) && i->isVisible()) page = i;
    QQuickItem *gain = page ? findItem(page, QStringLiteral("gainSection")) : nullptr;
    QQuickItem *gainSlider = page ? findItem(page, QStringLiteral("gainSectionSlider")) : nullptr;
    QQuickItem *delay = page ? findItem(page, QStringLiteral("delaySection")) : nullptr;
    QQuickItem *delaySlider = page ? findItem(page, QStringLiteral("delaySectionSlider")) : nullptr;
    check("QD41: the output page's gain and delay sections and sliders", gain && gainSlider && delay && delaySlider);
    if (!gain || !gainSlider || !delay || !delaySlider) return;
    SignalCounter gainCommits(gain, "committed(double)"), delayCommits(delay, "committed(double)");
    check("QD41: counting the sections' commits", gainCommits.ok && delayCommits.ok);
    // What the unsaved list says about output 1 (the preset's changes)
    const auto listed = [b](const char *what, const QString &to) {
        for (const QString &line : b->presetChanges())
            if (line.contains(QLatin1String(what)) && line.endsWith(to)) return true;
        return false;
    };

    // ── Gain: Right, then the key's release commits once ──
    b->setOutputGain(0, -3.0f);
    spin(100);
    gainSlider->forceActiveFocus();
    check("QD41: the gain slider has the keyboard", waitFor(1000, [&] { return main->activeFocusItem() == gainSlider; }));
    key(main, QEvent::KeyPress, Qt::Key_Right);
    check("QD41: Right sends the gain live", waitFor(1000, [&] { return near(b->outputGainDB(0), -2.9); }));
    check("QD41: and commits nothing while the key is down", gainCommits.count == 0);
    key(main, QEvent::KeyRelease, Qt::Key_Right);
    spin(50);
    check("QD41: the key's release commits the gain", gainCommits.count == 1 && near(b->outputGainDB(0), -2.9));
    check("QD41: the preset is dirty and the unsaved list shows the gain", b->presetDirty() && listed(" gain: ", QStringLiteral("-2.9 dB")));
    for (const QString &line : b->presetChanges()) printf("smoke: QD41 unsaved: %s\n", qPrintable(line));
    check("QD41: the gain field shows it", page->property("gainDB").toDouble() > -2.95 && page->property("gainDB").toDouble() < -2.85);

    // Held: auto-repeated releases don't commit (Qt's own Left/Right did,
    // each one), the last release does, once
    gainCommits.count = 0;
    key(main, QEvent::KeyPress, Qt::Key_Right);
    key(main, QEvent::KeyRelease, Qt::Key_Right, true);
    key(main, QEvent::KeyPress, Qt::Key_Right, true);
    key(main, QEvent::KeyRelease, Qt::Key_Right, true);
    key(main, QEvent::KeyPress, Qt::Key_Right, true);
    spin(50);
    check("QD41: a held Right steps the gain live", near(b->outputGainDB(0), -2.6));
    check("QD41: auto-repeat commits nothing", gainCommits.count == 0);
    key(main, QEvent::KeyRelease, Qt::Key_Right);
    spin(500);
    check("QD41: the final release commits once", gainCommits.count == 1 && near(b->outputGainDB(0), -2.6));

    // Up, Down, Home and End
    gainCommits.count = 0;
    tap(main, Qt::Key_Up);
    check("QD41: Up commits a tenth of a dB up", gainCommits.count == 1 && near(b->outputGainDB(0), -2.5));
    tap(main, Qt::Key_Down);
    check("QD41: Down commits a tenth of a dB down", gainCommits.count == 2 && near(b->outputGainDB(0), -2.6));
    tap(main, Qt::Key_End);
    check("QD41: End commits the top of the range (+12 dB)", gainCommits.count == 3 && near(b->outputGainDB(0), 12.0));
    tap(main, Qt::Key_End);
    check("QD41: End at the top commits nothing", gainCommits.count == 3);
    tap(main, Qt::Key_Home);
    check("QD41: Home commits the bottom (-60 dB)", gainCommits.count == 4 && near(b->outputGainDB(0), -60.0));

    // An accessibility increase sends live, then commits once after a pause
    b->setOutputGain(0, -3.0f);
    spin(100);
    gainCommits.count = 0;
    QAccessible::setActive(true);   // as for a screen reader (QD22 has it on already)
    QAccessibleInterface *ai = QAccessible::queryAccessibleInterface(gainSlider);
    QAccessibleActionInterface *act = ai ? ai->actionInterface() : nullptr;
    check("QD41: the gain slider's accessible actions", act && act->actionNames().contains(QAccessibleActionInterface::increaseAction()));
    if (act) {
        act->doAction(QAccessibleActionInterface::increaseAction());
        spin(100);
        act->doAction(QAccessibleActionInterface::increaseAction());
        check("QD41: accessibility increase sends the gain live", waitFor(1000, [&] { return near(b->outputGainDB(0), -2.8); }));
        check("QD41: not committed yet", gainCommits.count == 0);
        check("QD41: committed after a pause", waitFor(2000, [&] { return gainCommits.count == 1; }) && near(b->outputGainDB(0), -2.8));
        spin(600);
        check("QD41: once", gainCommits.count == 1 && listed(" gain: ", QStringLiteral("-2.8 dB")));
        act->doAction(QAccessibleActionInterface::decreaseAction());
        check("QD41: accessibility decrease commits too", waitFor(2000, [&] { return gainCommits.count == 2; }) && near(b->outputGainDB(0), -2.9));
    }

    // A pending accessibility change commits when the focus leaves
    gainCommits.count = 0;
    if (act) {
        act->doAction(QAccessibleActionInterface::increaseAction());
        spin(50);
        delaySlider->forceActiveFocus();
        spin(50);
        check("QD41: leaving the slider commits a pending change at once", gainCommits.count == 1 && near(b->outputGainDB(0), -2.8));
        spin(600);
        check("QD41: and the pause adds no second commit", gainCommits.count == 1);
    }

    // ── Delay: whole ms from a fractional start, as a drag (SL1) ──
    b->setOutputDelay(0, 0.25f);
    check("QD41: a fractional delay shown", waitFor(1000, [&] { return near(delaySlider->property("value").toDouble(), 0.25); }));
    delaySlider->forceActiveFocus();
    spin(50);
    key(main, QEvent::KeyPress, Qt::Key_Right);
    check("QD41: Right sends whole ms live", waitFor(1000, [&] { return near(b->outputDelayMS(0), 1.0); }));
    key(main, QEvent::KeyRelease, Qt::Key_Right);
    spin(50);
    printf("smoke: QD41 Right from 0.25 ms committed %.4f ms\n", double(b->outputDelayMS(0)));
    check("QD41: the release commits whole ms", delayCommits.count == 1 && near(b->outputDelayMS(0), 1.0) && whole(b->outputDelayMS(0)));
    check("QD41: the slider snaps to it", near(delaySlider->property("value").toDouble(), 1.0) && near(page->property("delayMS").toDouble(), 1.0));
    check("QD41: the unsaved list shows the delay", b->presetDirty() && listed(" delay: ", QStringLiteral("1 ms")));
    tap(main, Qt::Key_PageUp);
    const double page10 = std::max(1.0, std::round(std::ceil(double(b->maxDelayMs())) / 10));
    printf("smoke: QD41 PageUp from 1 ms committed %.4f ms (max %.1f)\n", double(b->outputDelayMS(0)), double(b->maxDelayMs()));
    check("QD41: PageUp commits a tenth of the range, whole ms", delayCommits.count == 2 && near(b->outputDelayMS(0), std::min<double>(b->maxDelayMs(), 1 + page10)) && whole(b->outputDelayMS(0)));
    tap(main, Qt::Key_PageDown);
    check("QD41: PageDown back", delayCommits.count == 3 && near(b->outputDelayMS(0), 1.0));
    tap(main, Qt::Key_Home);
    check("QD41: Home commits 0 ms", delayCommits.count == 4 && b->outputDelayMS(0) == 0.0f);
    b->setOutputDelay(0, 2.5f);
    check("QD41: 2.5 ms shown", waitFor(1000, [&] { return near(delaySlider->property("value").toDouble(), 2.5); }));
    delayCommits.count = 0;
    ai = QAccessible::queryAccessibleInterface(delaySlider);
    act = ai ? ai->actionInterface() : nullptr;
    if (act) {
        act->doAction(QAccessibleActionInterface::decreaseAction());
        check("QD41: accessibility decrease sends whole ms live", waitFor(1000, [&] { return near(b->outputDelayMS(0), 2.0); }));
        check("QD41: and commits it once after a pause", waitFor(2000, [&] { return delayCommits.count == 1; }) && near(b->outputDelayMS(0), 2.0));
        spin(600);
        check("QD41: once (delay)", delayCommits.count == 1 && listed(" delay: ", QStringLiteral("2 ms")));
    } else {
        check("QD41: the delay slider's accessible actions", false);
    }
    // Mouse behaviour is SL1's drag test (testDelaySliders): unchanged

    // ── The sidebar's volume slider (a commit from settled() too) ──
    QQuickItem *volume = nullptr;
    for (QQuickItem *i : root()->findChildren<QQuickItem *>())
        if (i->property("masterUnits").isValid() && i->metaObject()->indexOfSignal("settled()") >= 0) volume = i;
    check("QD41: the sidebar's volume slider", volume != nullptr);
    if (volume) {
        SignalCounter settles(volume, "settled()");
        volume->forceActiveFocus();
        spin(50);
        key(main, QEvent::KeyPress, Qt::Key_Left);
        key(main, QEvent::KeyRelease, Qt::Key_Left, true);
        key(main, QEvent::KeyPress, Qt::Key_Left, true);
        spin(50);
        check("QD41: volume: auto-repeat settles nothing", settles.ok && settles.count == 0);
        key(main, QEvent::KeyRelease, Qt::Key_Left);
        check("QD41: volume: the release settles once", settles.count == 1);
        tap(main, Qt::Key_Right);
        tap(main, Qt::Key_Right);
        check("QD41: volume: each tap settles", settles.count == 3);
    }

    main->contentItem()->forceActiveFocus();
    b->setOutputGain(0, gainBefore);
    b->setOutputDelay(0, delayBefore);
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(100);
}

#include "SmokeDriverSliderKeys.moc"
