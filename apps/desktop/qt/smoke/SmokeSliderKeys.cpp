// -- WP QP19 -- CustomSlider from the keyboard (carried from QD41): Left and
// Right move it and commit once per key, on the release as a drag does, and a
// held key's repeats commit once. On the base's probe window, so no device
// value changes. Up/Down and an accessibility increase are printed for the
// record: QD41 had them commit too, and today's component does not (QP18).
#include "SmokeDriver.h"

#include "Shell.h"

#include <QAccessible>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTest>
#include <cstdio>

namespace {

void sliderKeysSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    shell->trigger(QStringLiteral("smoke.window"));
    QQuickWindow *w = nullptr;
    d.waitFor(3000, [&] {
        w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("smoke.window")));
        return w && w->isVisible();
    });
    QQuickItem *slider = w ? d.item(QStringLiteral("SmokeProbeWindow.slider"), w->contentItem()) : nullptr;
    d.check("qp19-keys: the probe window's slider", slider != nullptr);
    if (!slider) return;

    QSignalSpy committed(slider, SIGNAL(committed(double)));
    d.check("qp19-keys: CustomSlider has committed(real)", committed.isValid());
    if (!committed.isValid()) return;
    const auto commits = [&] { return int(committed.count()); };
    const auto last = [&] { return committed.isEmpty() ? qQNaN() : committed.last().at(0).toDouble(); };

    slider->setProperty("value", 0.0);
    slider->forceActiveFocus();
    d.spin(50);
    const double from = slider->property("from").toDouble(), to = slider->property("to").toDouble();
    struct Step {
        Qt::Key key;
        const char *name;
    };
    // A horizontal Basic Slider's keys
    for (const Step &s : { Step{ Qt::Key_Right, "Right" }, Step{ Qt::Key_Left, "Left" } }) {
        const double before = slider->property("value").toDouble();
        const int commitsBefore = commits();
        QTest::keyClick(w, s.key);
        d.spin(30);
        const double after = slider->property("value").toDouble();
        d.check(QStringLiteral("qp19-keys: %1 moves the slider").arg(QLatin1String(s.name)), after != before && after >= from && after <= to);
        d.check(QStringLiteral("qp19-keys: %1 commits once, the value shown").arg(QLatin1String(s.name)),
                commits() == commitsBefore + 1 && last() == after);
    }

    // A held key: one press, two repeats, one release
    const int beforeRepeat = commits();
    for (int i = 0; i < 3; i++) {
        QKeyEvent press(QEvent::KeyPress, Qt::Key_Right, Qt::NoModifier, QString(), i > 0);
        QCoreApplication::sendEvent(slider, &press);
    }
    QKeyEvent release(QEvent::KeyRelease, Qt::Key_Right, Qt::NoModifier);
    QCoreApplication::sendEvent(slider, &release);
    d.spin(30);
    const int repeatCommits = commits() - beforeRepeat;
    d.check("qp19-keys: a held key commits once, on its release", repeatCommits == 1);

    // For the record
    const auto commitsFor = [&](Qt::Key key) {
        const int before = commits();
        QTest::keyClick(w, key);
        d.spin(30);
        return commits() - before;
    };
    const int up = commitsFor(Qt::Key_Up), down = commitsFor(Qt::Key_Down);
    int accessibleCommits = -1;
    if (QAccessibleInterface *a = QAccessible::queryAccessibleInterface(slider)) {
        if (QAccessibleActionInterface *act = a->actionInterface()) {
            const int beforeAction = commits();
            act->doAction(QAccessibleActionInterface::increaseAction());
            d.spin(500);
            accessibleCommits = commits() - beforeAction;
        }
    }
    std::printf("smoke: qp19-keys: Up commits %d, Down %d, an accessibility increase %d (QD41 had 1 each)\n", up, down,
                accessibleCommits);
    w->close();
}

const SmokeDriver::Section sliderKeys("qp19-slider-keys", 15, &sliderKeysSection);

} // namespace
