// -- WP QP0 -- The base components (fixup g0): each is made, carries its
// accessible role and name, and does what a person does with it.
#include "SmokeDriver.h"

#include "Shell.h"

#include <QAccessible>
#include <QQmlContext>
#include <QQmlProperty>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>

namespace {

QVariant attached(QObject *o, const char *name) {
    return QQmlProperty(o, QString::fromLatin1(name), qmlContext(o)).read();
}

// The item's Accessible role and a non-empty name
void checkAccessible(SmokeDriver &d, QObject *o, const QString &what, QAccessible::Role role) {
    d.check(QStringLiteral("qp0: %1 exists").arg(what), o != nullptr);
    if (!o) return;
    d.check(QStringLiteral("qp0: %1 has its accessible role").arg(what), attached(o, "Accessible.role").toInt() == int(role));
    d.check(QStringLiteral("qp0: %1 has an accessible name").arg(what), !attached(o, "Accessible.name").toString().isEmpty());
}

QObject *child(QQuickWindow *w, const char *name) {
    return w ? w->findChild<QObject *>(QString::fromLatin1(name)) : nullptr;
}

void componentsSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("smoke.window")));
    d.check(QStringLiteral("qp0: the probe window opens"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    w->requestActivate();
    d.spin(300);

    QObject *button = child(w, "SmokeProbeWindow.button");
    checkAccessible(d, button, QStringLiteral("AppButton"), QAccessible::Button);
    if (button) QMetaObject::invokeMethod(button, "click");
    d.check(QStringLiteral("qp0: AppButton clicks"), w->property("buttonClicks").toInt() == 1);

    QObject *sw = child(w, "SmokeProbeWindow.switch");
    checkAccessible(d, sw, QStringLiteral("AppSwitch"), QAccessible::CheckBox);
    if (sw) QMetaObject::invokeMethod(sw, "toggle");
    d.check(QStringLiteral("qp0: AppSwitch toggles"), sw && sw->property("checked").toBool());

    QObject *box = child(w, "SmokeProbeWindow.checkBox");
    checkAccessible(d, box, QStringLiteral("AppCheckBox"), QAccessible::CheckBox);
    if (box) QMetaObject::invokeMethod(box, "toggle");
    d.check(QStringLiteral("qp0: AppCheckBox toggles"), box && box->property("checked").toBool());

    QObject *combo = child(w, "SmokeProbeWindow.comboBox");
    checkAccessible(d, combo, QStringLiteral("AppComboBox"), QAccessible::ComboBox);
    if (combo) QMetaObject::invokeMethod(combo, "incrementCurrentIndex");
    d.check(QStringLiteral("qp0: AppComboBox chooses"), combo && combo->property("currentText").toString() == QLatin1String("S/PDIF"));

    QObject *spin = child(w, "SmokeProbeWindow.spinBox");
    checkAccessible(d, spin, QStringLiteral("AppSpinBox"), QAccessible::SpinBox);
    if (auto *spinItem = qobject_cast<QQuickItem *>(spin)) {
        spinItem->forceActiveFocus();
        d.spin(50);
        QTest::keyClick(w, Qt::Key_Up);
        d.spin(50);
    }
    d.check(QStringLiteral("qp0: AppSpinBox steps by its unit"), spin && qAbs(spin->property("realValue").toDouble() - 0.5) < 1e-9);
    d.check(QStringLiteral("qp0: AppSpinBox commits a step"), qAbs(w->property("spinCommitted").toDouble() - 0.5) < 1e-9);
    d.check(QStringLiteral("qp0: AppSpinBox shows the unit"), spin && spin->property("displayText").toString() == QLatin1String("0.5 ms"));

    QObject *tabs = child(w, "SmokeProbeWindow.tabBar");
    checkAccessible(d, tabs, QStringLiteral("AppTabBar"), QAccessible::PageTabList);
    QObject *tab2 = child(w, "SmokeProbeWindow.tab2");
    checkAccessible(d, tab2, QStringLiteral("AppTabButton"), QAccessible::PageTab);
    if (tab2) QMetaObject::invokeMethod(tab2, "click");
    d.check(QStringLiteral("qp0: AppTabBar switches"), tabs && tabs->property("currentIndex").toInt() == 1);

    // -- WP QPI1 -- CustomSlider: a right-click resets, a left click still moves
    auto *slider = qobject_cast<QQuickItem *>(child(w, "SmokeProbeWindow.slider"));
    checkAccessible(d, slider, QStringLiteral("CustomSlider"), QAccessible::Slider);
    if (slider) {
        const QPoint quarter = slider->mapToScene(QPointF(slider->width() * 0.25, slider->height() / 2)).toPoint();
        slider->setProperty("value", 12.0);
        QTest::mouseClick(w, Qt::RightButton, Qt::NoModifier, quarter);
        d.spin(50);
        d.check(QStringLiteral("qpi1: a right-click resets CustomSlider to its resetValue"),
                qAbs(slider->property("value").toDouble()) < 1e-9);
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, quarter);
        d.spin(50);
        d.check(QStringLiteral("qpi1: a left click still moves CustomSlider"), slider->property("value").toDouble() < -6.0);
        slider->setProperty("value", 0.0);
    }

    QObject *bar = child(w, "SmokeProbeWindow.scrollBar");
    checkAccessible(d, bar, QStringLiteral("AppScrollBar"), QAccessible::ScrollBar);
    d.check(QStringLiteral("qp0: AppScrollBar has something to scroll"), bar && bar->property("size").toDouble() < 1.0);

    QObject *heading = child(w, "SmokeProbeWindow.heading");
    d.check(QStringLiteral("qp0: SectionHeading exists"), heading != nullptr);
    if (heading) {
        bool found = false;
        for (QQuickItem *c : qobject_cast<QQuickItem *>(heading)->childItems())
            if (attached(c, "Accessible.role").toInt() == int(QAccessible::Heading) && attached(c, "Accessible.name").toString() == QLatin1String("Components")) found = true;
        d.check(QStringLiteral("qp0: SectionHeading is a heading"), found);
    }

    d.check(QStringLiteral("qp0: screenshot of the components"), d.grab(w, QStringLiteral("window-components.png")));

    // The question: Escape answers "reject", the default button "accept"
    QObject *question = child(w, "SmokeProbeWindow.question");
    checkAccessible(d, question ? question->property("contentItem").value<QObject *>() : nullptr,
                    QStringLiteral("QuestionDialog"), QAccessible::AlertMessage);
    QMetaObject::invokeMethod(w, "ask");
    d.check(QStringLiteral("qp0: QuestionDialog opens"), question && d.waitFor(2000, [&]() { return question->property("opened").toBool(); }));
    d.spin(200);
    d.check(QStringLiteral("qp0: screenshot of the question"), d.grab(w, QStringLiteral("window-question.png")));
    QTest::keyClick(w, Qt::Key_Escape);
    d.check(QStringLiteral("qp0: Escape answers reject"),
            d.waitFor(2000, [&]() { return w->property("lastAnswer").toString() == QLatin1String("reject"); }));
    QMetaObject::invokeMethod(w, "ask");
    d.waitFor(2000, [&]() { return question->property("opened").toBool(); });
    d.spin(100);
    QTest::keyClick(w, Qt::Key_Return);
    d.check(QStringLiteral("qp0: Enter answers accept"),
            d.waitFor(2000, [&]() { return w->property("lastAnswer").toString() == QLatin1String("accept"); }));

    QObject *popover = child(w, "SmokeProbeWindow.popover");
    checkAccessible(d, popover ? popover->property("background").value<QObject *>() : nullptr,
                    QStringLiteral("PopoverPanel"), QAccessible::Dialog);
    QMetaObject::invokeMethod(w, "showPopover");
    d.check(QStringLiteral("qp0: PopoverPanel opens"), popover && d.waitFor(2000, [&]() { return popover->property("opened").toBool(); }));
    QTest::keyClick(w, Qt::Key_Escape);
    d.check(QStringLiteral("qp0: Escape closes PopoverPanel"), popover && d.waitFor(2000, [&]() { return !popover->property("visible").toBool(); }));

    // The menu table's shortcuts work in a tool window too
    w->requestActivate();
    d.spin(100);
    // -- WP QP7 -- the shell's triggered signal: the row has a window now (QP7)
    QString reached;
    const auto seen = QObject::connect(shell, &Shell::triggered, shell, [&](const QString &id) { reached = id; });
    QTest::keyClick(w, Qt::Key_L, Qt::ControlModifier | Qt::ShiftModifier);
    d.check(QStringLiteral("qp0: Ctrl+Shift+L in a tool window reaches the shell"),
            d.waitFor(2000, [&]() { return reached == QLatin1String("tools.loudness"); }));
    QObject::disconnect(seen);
    if (QWindow *opened = shell->openWindow(QStringLiteral("tools.loudness"))) opened->close();
    w->close();
    // -- WP QP7 -- the main window active again for the sections after this one
    d.mainWindow()->requestActivate();
    d.spin(100);
}

const SmokeDriver::Section components("qp0-components", 2, &componentsSection);

} // namespace
