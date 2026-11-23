// -- WP USR1 -- Two bugs a user reported on the released Windows console
// (0.14.0, Classic), on the demo device:
// - "usr1-fields": a number typed into a field and left with Tab or a click on
//   another field is kept (it went back to the old value: a handler of the
//   field put the shown text back before the field committed what was typed),
//   as with Enter; Escape drops it. Typed as a person types, key by key, under
//   en_US, sv_SE and de_DE, with a decimal comma, the minus sign U+2212 and the
//   unit after the number. The main window's output gain, input mix gain and
//   delay, a band's frequency, gain and Q, and the matrix mixer's crosspoint
//   gain, output gain and delay.
// - "usr1-preset": the preset box shows the active preset, not the first one,
//   (0.14.1: the Classic layout, the only one) after every way the preset or its list changes: a change
//   that marks it unsaved, save, a load from the box, a load from elsewhere
//   (another client, the device), a copy into an empty slot, rename, a list
//   refresh and a reconnect.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Confirm.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "PresetModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QKeyEvent>
#include <QLocale>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <cmath>
#include <functional>

namespace {

bool near(double a, double b, double tol = 1e-3) { return std::abs(a - b) <= tol; }

QPoint centreOf(QQuickItem *i) { return i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint(); }

// The shown item named `name` under `from`
QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) {
        bool shown = true;
        for (QQuickItem *i = from; i && shown; i = i->parentItem()) shown = i->isVisible();
        if (shown) return from;
    }
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

// One key as a keyboard sends it: its key code and its text
void key(QQuickWindow *w, QChar c) {
    Qt::Key k = Qt::Key_unknown;
    if (c.isDigit()) k = Qt::Key(Qt::Key_0 + c.digitValue());
    else if (c.isLetter() && c.unicode() < 128) k = Qt::Key(Qt::Key_A + (c.toUpper().unicode() - 'A'));
    else if (c == QLatin1Char('-')) k = Qt::Key_Minus;
    else if (c == QLatin1Char('+')) k = Qt::Key_Plus;
    else if (c == QLatin1Char('.')) k = Qt::Key_Period;
    else if (c == QLatin1Char(',')) k = Qt::Key_Comma;
    else if (c == QLatin1Char(' ')) k = Qt::Key_Space;
    QKeyEvent press(QEvent::KeyPress, k, Qt::NoModifier, QString(c));
    QKeyEvent release(QEvent::KeyRelease, k, Qt::NoModifier, QString(c));
    QCoreApplication::sendEvent(w, &press);
    QCoreApplication::sendEvent(w, &release);
}

enum class Leave { Enter, EnterClick, Tab, Click, Escape };
const char *leaveName(Leave l) {
    switch (l) {
    case Leave::Enter: return "Enter";
    case Leave::EnterClick: return "Enter, then a click on another field once the device has it";
    case Leave::Tab: return "Tab";
    case Leave::Click: return "a click on another field";
    case Leave::Escape: return "Escape";
    }
    return "";
}

struct Field {
    QString what;                   // for the check's text
    QString name;                   // the text field's object name
    QString other;                  // a field to click into after typing
    std::function<double()> device; // what the device has
    std::function<void()> reset;    // the value before each entry
    double resetTo;
};

// Clicks into `f`, selects its text, types `text` key by key and leaves the
// field with `leave`; true when the device then has `expected`
bool enter(SmokeDriver &d, QQuickWindow *w, const Field &f, const QString &text, Leave leave, double expected) {
    f.reset();
    d.waitIdle();
    d.spin(30);
    QQuickItem *field = shownItem(w->contentItem(), f.name);
    QQuickItem *other = shownItem(w->contentItem(), f.other);
    if (!field || !other) {
        d.check(QStringLiteral("usr1: %1 and %2 are on screen").arg(f.name, f.other), false);
        return false;
    }
    other->forceActiveFocus();
    d.spin(20);
    QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, centreOf(field));
    d.spin(30);
    if (!field->hasActiveFocus()) {
        d.check(QStringLiteral("usr1: a click gives %1 the focus").arg(f.what), false);
        return false;
    }
    QTest::keyClick(w, Qt::Key_A, Qt::ControlModifier);
    for (const QChar c : text) key(w, c);
    d.spin(20);
    switch (leave) {
    case Leave::Enter: QTest::keyClick(w, Qt::Key_Return); break;
    case Leave::EnterClick:
        // The user's report: the value went in on Enter, and back out on the
        // click that followed
        QTest::keyClick(w, Qt::Key_Return);
        d.spin(30);
        d.waitIdle();
        d.spin(60);
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, centreOf(other));
        break;
    case Leave::Tab: QTest::keyClick(w, Qt::Key_Tab); break;
    case Leave::Click: QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, centreOf(other)); break;
    case Leave::Escape: QTest::keyClick(w, Qt::Key_Escape); break;
    }
    d.spin(30);
    d.waitIdle();
    d.spin(60);
    const double got = f.device();
    const bool ok = near(got, expected, 1e-3);
    if (!ok)
        std::printf("smoke: usr1: %s: typed \"%s\", left with %s: the device has %g, not %g\n", qPrintable(f.what),
                    qPrintable(text), leaveName(leave), got, expected);
    return ok;
}

// Every entry form and every way out, under each locale
void fieldChecks(SmokeDriver &d, QQuickWindow *w, const Field &f, const QStringList &texts, double expected) {
    const QLocale before;
    for (const char *locale : { "en_US", "sv_SE", "de_DE" }) {
        QLocale::setDefault(QLocale(QString::fromLatin1(locale)));
        for (Leave leave : { Leave::Enter, Leave::EnterClick, Leave::Tab, Leave::Click }) {
            bool all = true;
            for (const QString &t : texts) all = enter(d, w, f, t, leave, expected) && all;
            d.check(QStringLiteral("usr1: %1, %2: %3 typed and left with %4 is kept")
                        .arg(f.what, QString::fromLatin1(locale), texts.join(QStringLiteral(" / ")), QString::fromLatin1(leaveName(leave))),
                    all);
        }
        d.check(QStringLiteral("usr1: %1, %2: Escape drops what was typed").arg(f.what, QString::fromLatin1(locale)),
                enter(d, w, f, texts.first(), Leave::Escape, f.resetTo));
    }
    QLocale::setDefault(before);
}

// "-3.5" as people type it: a point, a comma, the minus sign, the unit after it
QStringList signedForms(const QString &number, const QString &unit) {
    QString comma = number;
    comma.replace(QLatin1Char('.'), QLatin1Char(','));
    QString minus = comma;
    minus.replace(QLatin1Char('-'), QChar(0x2212));
    return { number, comma, minus, number + QLatin1Char(' ') + unit };
}

void fieldsSection(SmokeDriver &d) {
    QQuickWindow *w = d.mainWindow();
    auto *sc = Registrar::instance<SidebarController>();
    auto *g = Registrar::instance<GraphController>();
    auto *cc = Registrar::instance<ChannelController>();
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("usr1: the main window and the controllers"), w && sc && g && cc);
    if (!w || !sc || !g || !cc) return;
    d.check(QStringLiteral("usr1: on the demo"), d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    d.allowSectionTime(300);   // every field, entry form and way out, under three locales
    const QSize original = w->size();
    w->resize(QSize(1600, 1000));
    const int ch = g->firstOutput();
    const int out = ch - cc->numInputs();
    sc->select(ch);
    d.waitFor(3000, [&] { return d.item(QStringLiteral("OutputDetail")) != nullptr; });
    d.spin(200);
    d.waitIdle();
    auto s = [&] { return device->state(); };

    // ---- The main window (Classic): the output card and the band list ----
    cc->setRoute(0, out, true, 0, false);
    cc->clearBands(ch, false);
    cc->setBandType(ch, 0, false, FILTER_PEAKING);
    d.waitIdle();
    const Field gain{ QStringLiteral("the output gain"), QStringLiteral("OutputCard.gainValue.field"),
                      QStringLiteral("OutputCard.delayValue.field"), [&] { return double(s()->output_gain_db[out]); },
                      [&] { cc->setOutputGain(out, 0); }, 0 };
    fieldChecks(d, w, gain, signedForms(QStringLiteral("-3.5"), QStringLiteral("dB")), -3.5);
    const Field route{ QStringLiteral("the input mix gain"), QStringLiteral("OutputCard.routeGain.0.field"),
                       QStringLiteral("OutputCard.gainValue.field"), [&] { return double(s()->matrix_gain[0][out]); },
                       [&] { cc->setRoute(0, out, true, 0, false); }, 0 };
    fieldChecks(d, w, route, signedForms(QStringLiteral("-6.5"), QStringLiteral("dB")), -6.5);
    const Field delay{ QStringLiteral("the output delay"), QStringLiteral("OutputCard.delayValue.field"),
                       QStringLiteral("OutputCard.gainValue.field"), [&] { return double(s()->output_delay_ms[out]); },
                       [&] { cc->setOutputDelay(out, 0); }, 0 };
    fieldChecks(d, w, delay, { QStringLiteral("2.5"), QStringLiteral("2,5"), QStringLiteral("2.5 ms") }, 2.5);
    const QString band = QStringLiteral("OutputDetail.bands.band.0.");
    const Field freq{ QStringLiteral("a band's frequency"), band + QStringLiteral("freq.field"), band + QStringLiteral("gain.field"),
                      [&] { return double(s()->filters[ch][0].freq); },
                      [&] { cc->setBandValue(ch, 0, false, QStringLiteral("freq"), 1000); }, 1000 };
    fieldChecks(d, w, freq, { QStringLiteral("1250.5"), QStringLiteral("1250,5"), QStringLiteral("1250.5 Hz") }, 1250.5);
    const Field bandGain{ QStringLiteral("a band's gain"), band + QStringLiteral("gain.field"), band + QStringLiteral("freq.field"),
                          [&] { return double(s()->filters[ch][0].gain); },
                          [&] { cc->setBandValue(ch, 0, false, QStringLiteral("gain"), 0); }, 0 };
    fieldChecks(d, w, bandGain, signedForms(QStringLiteral("-4.5"), QStringLiteral("dB")), -4.5);
    const Field q{ QStringLiteral("a band's Q"), band + QStringLiteral("q.field"), band + QStringLiteral("freq.field"),
                   [&] { return double(s()->filters[ch][0].q); }, [&] { cc->setBandValue(ch, 0, false, QStringLiteral("q"), 1); }, 1 };
    fieldChecks(d, w, q, { QStringLiteral("2.5"), QStringLiteral("2,5") }, 2.5);
    cc->clearBands(ch, false);
    cc->setOutputGain(out, 0);
    d.waitIdle();

    // ---- The matrix mixer ----
    Shell *shell = d.app().shell;
    shell->trigger(QStringLiteral("tools.matrix"));
    auto *mw = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("tools.matrix")));
    d.check(QStringLiteral("usr1: the matrix mixer opens"), mw && d.waitFor(3000, [&] { return mw->isVisible(); }));
    if (mw) {
        const QSize matrixSize = mw->size();
        mw->resize(QSize(1400, 900));
        cc->setRoute(1, 0, true, 0, false);
        d.waitIdle();
        d.spin(150);
        const Field cell{ QStringLiteral("a matrix crosspoint gain"), QStringLiteral("Matrix.cell.0.0.gain"),
                          QStringLiteral("Matrix.cell.1.0.gain"), [&] { return double(s()->matrix_gain[0][0]); },
                          [&] { cc->setRoute(0, 0, true, 0, false); }, 0 };
        fieldChecks(d, mw, cell, signedForms(QStringLiteral("-6.5"), QStringLiteral("dB")), -6.5);
        const Field outGain{ QStringLiteral("the matrix's output gain"), QStringLiteral("Matrix.gain.0"), QStringLiteral("Matrix.gain.1"),
                             [&] { return double(s()->output_gain_db[0]); }, [&] { cc->setOutputGain(0, 0); }, 0 };
        fieldChecks(d, mw, outGain, signedForms(QStringLiteral("-2.5"), QStringLiteral("dB")), -2.5);
        const Field outDelay{ QStringLiteral("the matrix's output delay"), QStringLiteral("Matrix.delay.0"), QStringLiteral("Matrix.delay.1"),
                              [&] { return double(s()->output_delay_ms[0]); }, [&] { cc->setOutputDelay(0, 0); }, 0 };
        fieldChecks(d, mw, outDelay, { QStringLiteral("1.5"), QStringLiteral("1,5"), QStringLiteral("1.5 ms") }, 1.5);
        cc->setOutputGain(0, 0);
        cc->setOutputDelay(0, 0);
        d.waitIdle();
        // No field keeps the focus for the window's next opening (its wheel steps only an unfocused field)
        mw->contentItem()->forceActiveFocus();
        mw->resize(matrixSize);   // qp6 aims its wheel by position in the same window
        mw->close();
    }

    w->contentItem()->forceActiveFocus();
    w->resize(original);
    sc->select(-1);
    d.spin(100);
}

const SmokeDriver::Section fields("usr1-fields", 37, &fieldsSection);

// ---- The preset box ----

struct Box {
    SmokeDriver &d;
    SidebarController *sc;
    DeviceModel *device;
    QString layout;
    QString name;   // GlobalControls.preset or TopBar.preset

    QQuickItem *box() const { return d.item(name); }
    int active() const { return device->presets()->activeSlot(); }
    QString shown() const { return box() ? box()->property("shownName").toString() : QString(); }
    // What the box should say: the active slot's title, with its "*"
    QString expected() const {
        const QStringList titles = sc->presetTitles();
        return active() >= 0 && active() < titles.size() ? titles.at(active()) : QString();
    }
    void check(const QString &after) {
        const bool ok = d.waitFor(2000, [&] { return !expected().isEmpty() && shown() == expected(); });
        if (!ok)
            std::printf("smoke: usr1: %s: after %s the box says \"%s\", the active slot %d is \"%s\"\n", qPrintable(layout),
                        qPrintable(after), qPrintable(shown()), active(), qPrintable(expected()));
        d.check(QStringLiteral("usr1: %1: the preset box shows the active preset after %2").arg(layout, after), ok);
    }
    bool idle() { return d.waitFor(8000, [&] { return !sc->presetBusy(); }) && d.waitIdle(); }
    void clean() {
        if (sc->unsaved()) sc->savePreset();
        idle();
        d.waitFor(3000, [&] { return !sc->unsaved(); });
    }
};

void presetBox(SmokeDriver &d, const QString &layout, const QString &name) {
    auto *sc = Registrar::instance<SidebarController>();
    DeviceModel *device = d.app().device;
    Box b{ d, sc, device, layout, name };
    d.check(QStringLiteral("usr1: %1: the preset box").arg(layout), b.box() != nullptr);
    if (!b.box()) return;
    b.clean();
    const int start = b.active();
    d.check(QStringLiteral("usr1: %1: the demo starts on slot 1 with nothing unsaved").arg(layout), start == 0 && !sc->unsaved());
    if (start != 0) return;
    // A second preset in slot 2, and an empty slot for the copy
    const bool made = !sc->presetOccupied(1);
    if (made) {
        sc->requestPresetCopy(1);
        b.idle();
    }
    int spare = -1;
    for (int s = 2; s < MAX_PRESETS && spare < 0; ++s)
        if (!sc->presetOccupied(s)) spare = s;
    d.check(QStringLiteral("usr1: %1: slot 2 is a preset and an empty slot is left").arg(layout), sc->presetOccupied(1) && spare > 1);
    if (!sc->presetOccupied(1) || spare < 0) return;
    const QString name2 = sc->presetTitle(1);

    // Load from the box, as a person does: the focus on it and Down
    b.box()->forceActiveFocus();
    QTest::keyClick(b.box()->window(), Qt::Key_Down);
    d.check(QStringLiteral("usr1: %1: Down on the box loads slot 2").arg(layout), d.waitFor(8000, [&] { return b.active() == 1; }) && b.idle());
    b.check(QStringLiteral("a load from the box"));

    // An edit: the active title gains its "*" (a new list of titles)
    const float gain = device->state()->output_gain_db[0];
    device->write(QStringLiteral("usr1 edit"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain - 2.0f); });
    d.check(QStringLiteral("usr1: %1: an edit marks slot 2 unsaved").arg(layout), d.waitFor(3000, [&] { return sc->unsaved(); }));
    b.check(QStringLiteral("an edit marks it unsaved"));
    d.check(QStringLiteral("usr1: %1: and the box carries the \"*\"").arg(layout), b.shown().endsWith(QLatin1Char('*')));

    // Save: the "*" goes (a new list again)
    sc->savePreset();
    b.idle();
    d.check(QStringLiteral("usr1: %1: Save leaves nothing unsaved").arg(layout), d.waitFor(3000, [&] { return !sc->unsaved(); }));
    b.check(QStringLiteral("Save"));

    // Saved as a new preset: a copy into an empty slot (Copy to…), the source stays active
    sc->requestPresetCopy(spare);
    d.check(QStringLiteral("usr1: %1: the copy fills slot %2").arg(layout).arg(spare + 1),
            b.idle() && d.waitFor(3000, [&] { return sc->presetOccupied(spare); }) && b.active() == 1);
    b.check(QStringLiteral("a copy into an empty slot"));

    // Rename
    sc->renamePreset(1, QStringLiteral("USR1 Second"));
    d.check(QStringLiteral("usr1: %1: Rename names slot 2").arg(layout),
            d.waitFor(3000, [&] { return sc->presetTitle(1) == QLatin1String("USR1 Second"); }));
    b.check(QStringLiteral("Rename"));

    // The list read again from the device
    device->refresh();
    b.idle();
    b.check(QStringLiteral("a refresh"));

    // A load from elsewhere (another client, an agent, the device's own button): not through the box
    device->write(QStringLiteral("usr1 load"), [](FfiCore *c) { return dspi_load_preset(c, 0) == PRESET_OK; });
    d.check(QStringLiteral("usr1: %1: a load from elsewhere reaches the app").arg(layout), d.waitFor(8000, [&] { return b.active() == 0; }) && b.idle());
    b.check(QStringLiteral("a load from elsewhere"));
    device->write(QStringLiteral("usr1 load"), [](FfiCore *c) { return dspi_load_preset(c, 1) == PRESET_OK; });
    d.waitFor(8000, [&] { return b.active() == 1; });
    b.idle();
    b.check(QStringLiteral("a second load from elsewhere"));

    // Reconnect (the state, the titles and the active slot come again)
    device->reconnect();
    d.check(QStringLiteral("usr1: %1: the demo reconnects").arg(layout),
            d.waitFor(10000, [&] { return device->connected() && b.active() >= 0; }) && b.idle());
    b.check(QStringLiteral("a reconnect"));

    // Back to how the demo was
    b.clean();
    if (b.active() != 0) {
        device->write(QStringLiteral("usr1 load"), [](FfiCore *c) { return dspi_load_preset(c, 0) == PRESET_OK; });
        d.waitFor(8000, [&] { return b.active() == 0; });
        b.idle();
    }
    device->write(QStringLiteral("usr1 gain"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain); });
    b.idle();
    b.clean();
    sc->clearPreset(spare);
    b.idle();
    // (a reconnect to the demo may have put its presets back already)
    if (made) sc->clearPreset(1);
    else if (sc->presetOccupied(1) && sc->presetTitle(1) != name2) sc->renamePreset(1, name2);
    b.idle();
    d.check(QStringLiteral("usr1: %1: the demo is back on slot 1").arg(layout), b.active() == 0 && !sc->presetOccupied(spare));
}

// The source box beside it, the same flaw: a renamed source is a new list
void sourceBox(SmokeDriver &d) {
    auto *sc = Registrar::instance<SidebarController>();
    QQuickItem *box = d.item(QStringLiteral("GlobalControls.source"));
    if (!sc->sourceNamesSupported()) {
        d.notRun(QStringLiteral("usr1: the source box after a rename"), QStringLiteral("no source names on this firmware"));
        return;
    }
    const int start = sc->source();
    const QString before = sc->sourceName(1);
    sc->setSource(1);
    d.waitFor(3000, [&] { return sc->source() == 1; });
    d.waitIdle();
    sc->renameSource(1, QStringLiteral("USR1 Optical"));
    d.waitFor(3000, [&] { return sc->sourceTitle(1) == QLatin1String("USR1 Optical"); });
    d.waitIdle();
    const bool ok = d.waitFor(2000, [&] { return box && box->property("shownName").toString() == sc->sourceTitle(1); });
    if (!ok && box) std::printf("smoke: usr1: the source box says \"%s\"\n", qPrintable(box->property("shownName").toString()));
    d.check(QStringLiteral("usr1: Classic: the source box shows the chosen source after it is renamed"), ok);
    sc->renameSource(1, before);
    sc->setSource(start);
    d.waitIdle();
}

void presetSection(SmokeDriver &d) {
    d.check(QStringLiteral("usr1: on the demo"), d.waitFor(10000, [&] { return d.app().device->connected(); }) && d.waitIdle());
    presetBox(d, QStringLiteral("Classic"), QStringLiteral("GlobalControls.preset"));
    sourceBox(d);
    // 0.14 has the Classic layout only
}

const SmokeDriver::Section preset("usr1-preset", 37, &presetSection);

} // namespace
