// -- WP UB13 -- (UXP B13, D11) Settings > Appearance > Accessibility: the
// opt-in "Outline controls" (a 1 px Theme.controlOutline edge on fields, combo
// boxes, steppers, buttons, segments and chips, in both layouts) and the
// Comfortable density (32 px controls, 28 px chips, a 20 px reach round a PEQ
// dot; Modern only), chosen or automatic on a touch screen. Off and Standard
// on a mouse desktop: the look stays the owner's.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "LayoutController.h"
#include "PeqEditorController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "render/PeqOverlay.h"

#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTest>
#include <cstdio>
#include <memory>

namespace {

const char kProbe[] = R"QML(
import QtQuick
import QtQuick.Layouts
import UConsole

Window {
    width: 520
    height: 200
    visible: true
    color: Theme.window
    title: "ub13"
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacing
        RowLayout {
            spacing: Theme.spacingSmall
            AppButton { objectName: "Ub13.button"; text: "Button" }
            AppButton { objectName: "Ub13.primary"; text: "Primary"; primary: true }
            BorderedField { objectName: "Ub13.field"; text: "Field"; Layout.preferredWidth: 90 }
            AppComboBox { objectName: "Ub13.combo"; model: ["Combo", "Two"]; Layout.preferredWidth: 100 }
        }
        RowLayout {
            spacing: Theme.spacingSmall
            AppSpinBox { objectName: "Ub13.spin"; realFrom: 0; realTo: 9; realValue: 3 }
            NumericField { objectName: "Ub13.number"; from: 20; to: 20000; value: 1000; decimals: 0; Layout.preferredWidth: 88 }
            OptionChip { objectName: "Ub13.chip"; text: "Chip" }
            ToolSegments { objectName: "Ub13.segments"; model: ["One", "Two"]; current: 0 }
        }
    }
}
)QML";

QObject *background(QObject *control) {
    return control ? control->property("background").value<QObject *>() : nullptr;
}
QObject *borderOf(QObject *rect) { return rect ? rect->property("border").value<QObject *>() : nullptr; }
int borderWidth(QObject *rect) {
    QObject *b = borderOf(rect);
    return b ? b->property("width").toInt() : -1;
}
QColor borderColor(QObject *rect) {
    QObject *b = borderOf(rect);
    return b ? b->property("color").value<QColor>() : QColor();
}
// ToolSegments' bar is its first child
QObject *segmentBar(QQuickItem *segments) {
    return segments && !segments->childItems().isEmpty() ? segments->childItems().first() : nullptr;
}

struct Probe {
    SmokeDriver &d;
    QObject *theme;
    QQuickWindow *w = nullptr;

    QQuickItem *item(const char *name) const { return d.item(QString::fromLatin1(name), w->contentItem()); }
    QObject *edge(const char *name) const {
        QQuickItem *i = item(name);
        return QString::fromLatin1(name) == QLatin1String("Ub13.segments") ? segmentBar(i) : background(i);
    }
    // Every control's edge `width` wide, in `color` when it has one
    bool edges(int width, const QColor &color) const {
        bool ok = true;
        for (const char *name : { "Ub13.button", "Ub13.field", "Ub13.combo", "Ub13.spin", "Ub13.number", "Ub13.segments" }) {
            QObject *e = edge(name);
            const bool good = e && borderWidth(e) == width && (width == 0 || borderColor(e) == color);
            if (!good)
                std::printf("smoke: ub13: %s edge %d %s, wanted %d %s\n", name, borderWidth(e), qPrintable(borderColor(e).name()),
                            width, qPrintable(color.name()));
            ok = ok && good;
        }
        return ok;
    }
    bool heights(int control, int chip) const {
        bool ok = true;
        for (const char *name : { "Ub13.button", "Ub13.field", "Ub13.combo", "Ub13.spin", "Ub13.number", "Ub13.segments" }) {
            QQuickItem *i = item(name);
            ok = ok && i && qAbs(i->height() - control) < 0.5;
            if (i && qAbs(i->height() - control) >= 0.5) std::printf("smoke: ub13: %s %.0f high, wanted %d\n", name, i->height(), control);
        }
        QQuickItem *c = item("Ub13.chip");
        return ok && c && qAbs(c->height() - chip) < 0.5;
    }
    QColor color(const char *token) const { return theme->property(token).value<QColor>(); }
    int number(const char *token) const { return theme->property(token).toInt(); }
};

// The PEQ page's dot reaches 10 px (Standard) or 20 px (Comfortable): a
// pointer 15 px above a peak's dot takes it only in Comfortable
void peqReach(SmokeDriver &d, LayoutController *layout, QObject *theme) {
    auto *cc = Registrar::instance<ChannelController>();
    auto *ed = Registrar::instance<PeqEditorController>();
    DeviceModel *device = d.app().device;
    QQuickWindow *main = d.mainWindow();
    d.check("ub13: the PEQ page's controllers", cc && ed && main);
    if (!cc || !ed || !main) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    main->resize(1280, 800);
    d.goTo(QStringLiteral("equaliser/peq"), 300);
    auto *ov = qobject_cast<PeqOverlay *>(d.item(QStringLiteral("PeqEditor.overlay")));
    const int ch = ov ? ov->editedChannel() : -1;
    d.check("ub13: the PEQ page edits a channel", ov && ch >= 0);
    if (!ov || ch < 0) return;
    QList<QPair<int, FilterParams>> back;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) back << qMakePair(b, device->state()->filters[ch][b]);
    cc->clearBands(ch, false);
    cc->setBandType(ch, 0, false, FILTER_PEAKING);
    cc->setBandValue(ch, 0, false, QStringLiteral("freq"), 1000);
    cc->setBandValue(ch, 0, false, QStringLiteral("q"), 1);
    cc->setBandValue(ch, 0, false, QStringLiteral("gain"), 6);
    if (cc->bypassSupported()) cc->setBandBypass(ch, 0, false, false);
    d.waitIdle();
    d.spin(60);
    ov->settle();
    const auto reaches = [&] {
        const QPoint dot = ov->mapToScene(ov->dotPoint(0)).toPoint();
        QTest::mouseMove(main, dot + QPoint(0, 60));
        d.spin(40);
        QTest::mouseMove(main, dot + QPoint(0, -15));
        d.spin(40);
        const bool taken = ov->pointedBand() == 0;
        QTest::mouseMove(main, dot + QPoint(0, 60));
        d.spin(40);
        return taken;
    };
    layout->setDensity(QStringLiteral("standard"));
    d.spin(60);
    d.check("ub13: Standard: the dot reaches 10 px (Theme.peqHitRadius, the overlay's dotHit)",
            theme->property("peqHitRadius").toDouble() == 10 && ov->property("dotHit").toDouble() == 10);
    d.check("ub13: Standard: a pointer 15 px above the dot does not take it", !reaches());
    layout->setDensity(QStringLiteral("comfortable"));
    d.spin(60);
    d.check("ub13: Comfortable: the dot reaches 20 px",
            theme->property("peqHitRadius").toDouble() == 20 && ov->property("dotHit").toDouble() == 20);
    d.check("ub13: Comfortable: a pointer 15 px above the dot takes it", reaches());
    d.grab(main, QStringLiteral("ub13-peq-comfortable.png"));
    layout->setDensity(QStringLiteral("automatic"));
    ed->writeBands(ch, back);
    d.waitIdle();
}

void ub13Section(SmokeDriver &d) {
    auto *layout = Registrar::instance<LayoutController>();
    QObject *theme = d.app().engine->singletonInstance<QObject *>("UConsole", "Theme");
    d.check("ub13: the theme", layout && theme);
    if (!layout || !theme) return;
    const QString before = layout->layout();

    // A mouse desktop: both off
    d.check("ub13: off by default: no outline, Automatic, no touch screen here",
            !layout->outlineControls() && layout->density() == QLatin1String("automatic") && !layout->touchScreen()
            && !layout->comfortable());
    d.check("ub13: Automatic is Comfortable with a touch screen and Standard without",
            LayoutController::comfortableFor(QStringLiteral("automatic"), true)
            && !LayoutController::comfortableFor(QStringLiteral("automatic"), false)
            && LayoutController::comfortableFor(QStringLiteral("comfortable"), false)
            && !LayoutController::comfortableFor(QStringLiteral("standard"), true));
    layout->setDensity(QStringLiteral("large"));
    d.check("ub13: an unknown density is ignored", layout->density() == QLatin1String("automatic"));

    d.check("ub13: in Modern", d.switchLayout("modern"));
    QQmlComponent component(d.app().engine);
    component.setData(QByteArray(kProbe), QUrl(QStringLiteral("ub13.qml")));
    std::unique_ptr<QObject> made(component.create());
    if (!made) std::printf("smoke: ub13: %s\n", qPrintable(component.errorString()));
    Probe p{ d, theme, qobject_cast<QQuickWindow *>(made.get()) };
    d.check("ub13: the probe window", p.w != nullptr);
    if (!p.w) return;
    d.waitFor(3000, [&] { return p.w->isExposed(); });
    d.spin(200);

    const QString scheme = Shell::instance() && Shell::instance()->darkScheme() ? QStringLiteral("dark") : QStringLiteral("light");
    d.check("ub13: Modern, off: no outline, 24 px controls and chips (the owner's look)",
            p.edges(0, QColor()) && p.heights(24, 24) && p.number("controlLineWidth") == 0 && p.number("chipLineWidth") == 0
            && borderWidth(background(p.item("Ub13.chip"))) == 0);
    d.grab(p.w, QStringLiteral("ub13-off-%1.png").arg(scheme));

    // Outline controls
    layout->setOutlineControls(true);
    d.spin(60);
    const QColor outline = p.color("controlOutline");
    d.check("ub13: Outline controls: a 1 px controlOutline edge on every field, combo box, stepper, button and segment bar",
            p.edges(1, outline));
    d.check("ub13: and on a chip", borderWidth(background(p.item("Ub13.chip"))) == 1
                                       && borderColor(background(p.item("Ub13.chip"))) == outline);
    d.check("ub13: the primary button keeps its fill alone", borderWidth(background(p.item("Ub13.primary"))) == 0);
    d.check("ub13: the setting is kept", QSettings().value(QLatin1String(LayoutController::kOutlineKey)).toBool());
    p.item("Ub13.field")->forceActiveFocus(Qt::TabFocusReason);
    d.spin(40);
    d.check("ub13: focus still shows the 2 px focus ring", borderWidth(background(p.item("Ub13.field"))) == 2
                                                             && borderColor(background(p.item("Ub13.field"))) == p.color("focusRing"));
    p.item("Ub13.button")->forceActiveFocus(Qt::OtherFocusReason);
    d.spin(40);
    d.grab(p.w, QStringLiteral("ub13-outline-%1.png").arg(scheme));

    // Comfortable
    layout->setDensity(QStringLiteral("comfortable"));
    d.spin(100);
    d.check("ub13: Comfortable: 32 px controls, 28 px chips", layout->comfortable() && p.heights(32, 28));
    d.check("ub13: the density is kept", QSettings().value(QLatin1String(LayoutController::kDensityKey)).toString() == QLatin1String("comfortable"));
    d.grab(p.w, QStringLiteral("ub13-comfortable-%1.png").arg(scheme));
    layout->setOutlineControls(false);
    d.spin(60);
    d.check("ub13: Comfortable without the outline: no edge", p.edges(0, QColor()) && p.heights(32, 28));

    // Classic keeps the Mac console's sizes; the outline only changes its line's colour
    d.check("ub13: in Classic", d.switchLayout("classic"));
    d.spin(100);
    d.check("ub13: Classic ignores the density: 24 px", p.number("controlHeight") == 24 && p.heights(24, 24));
    d.check("ub13: Classic, off: its 1 px border", p.edges(1, p.color("border")));
    layout->setOutlineControls(true);
    d.spin(60);
    d.check("ub13: Classic, Outline controls: the line in controlOutline", p.edges(1, p.color("controlOutline")));
    layout->setOutlineControls(false);
    layout->setDensity(QStringLiteral("standard"));
    d.check("ub13: back in Modern", d.switchLayout("modern"));
    d.spin(60);
    d.check("ub13: Standard chosen: 24 px", p.heights(24, 24));
    p.w->close();
    made.reset();

    // The settings themselves, on Settings > Appearance
    d.app().shell->trigger(QStringLiteral("edit.settings"));
    Registrar::instance<SettingsController>()->open(QStringLiteral("appearance"));
    d.spin(300);
    QQuickItem *outlineRow = d.item(QStringLiteral("Appearance.outline"));
    QQuickItem *densityRow = d.item(QStringLiteral("Appearance.density"));
    d.check("ub13: Settings > Appearance shows Outline controls and Control size", outlineRow && densityRow
            && outlineRow->isVisible() && densityRow->isVisible() && densityRow->isEnabled());
    if (outlineRow) {
        QMetaObject::invokeMethod(outlineRow, "toggled", Q_ARG(bool, true));
        d.check("ub13: its switch turns the outline on", layout->outlineControls());
        d.check("ub13: and the switch follows", outlineRow->property("checked").toBool());
    }
    if (densityRow) {
        QMetaObject::invokeMethod(densityRow, "chosen", Q_ARG(QVariant, QStringLiteral("comfortable")));
        d.check("ub13: its picker sets the density", layout->density() == QLatin1String("comfortable"));
    }
    if (QQuickWindow *main = d.mainWindow()) {
        d.spin(200);
        d.grab(main, QStringLiteral("ub13-settings-%1.png").arg(scheme));
    }
    layout->setOutlineControls(false);
    layout->setDensity(QStringLiteral("automatic"));
    d.app().shell->trigger(QStringLiteral("edit.settings"));   // closes it again
    d.spin(100);

    peqReach(d, layout, theme);

    layout->setOutlineControls(false);
    layout->setDensity(QStringLiteral("automatic"));
    d.check("ub13: put back: off and Automatic", !layout->outlineControls() && layout->density() == QLatin1String("automatic")
                                                     && theme->property("controlHeight").toInt() == 24);
    d.switchLayout(before == QLatin1String("modern") ? "modern" : "classic");
}

const SmokeDriver::Section ub13("ub13-density", 986, &ub13Section);

} // namespace
