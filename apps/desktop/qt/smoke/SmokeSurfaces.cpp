// -- WP QP12 -- The Control pages on the demo device: Control Surfaces (cards,
// the editor rows from the core, Apply and Revert, a rename, adding and
// removing a control, a refused apply), the IR receiver's remote buttons and
// the learn, the display's settings and pages, Auxiliary Outputs (the live
// switch and level), Channel Groups and Macros, the save bar's part, and the
// pins the bindings hold. Everything changed on the demo is put back.
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreWorker.h"
#include "CsController.h"
#include "DeviceModel.h"
#include "HardwareController.h"
#include "SettingsController.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QQuickItem>
#include <QQuickWindow>
#include <algorithm>
#include <cmath>

namespace {

QObject *qp12Find(QObject *root, const QString &name) {
    if (!root) return nullptr;
    if (root->objectName() == name) return root;
    if (auto *item = qobject_cast<QQuickItem *>(root))
        for (QQuickItem *child : item->childItems())
            if (QObject *o = qp12Find(child, name)) return o;
    for (QObject *child : root->children())
        if (QObject *o = qp12Find(child, name)) return o;
    return nullptr;
}

struct Qp12Ui {
    SmokeDriver &d;
    QQuickWindow *w;
    QObject *get(const QString &name) const { return w ? qp12Find(w->contentItem(), name) : nullptr; }
    bool shown(const QString &name) const {
        auto *item = qobject_cast<QQuickItem *>(get(name));
        return item && item->isVisible();
    }
    QString text(const QString &name) const {
        QObject *o = get(name);
        return o ? o->property("text").toString() : QString();
    }
    bool click(const QString &name) const {
        QObject *o = get(name);
        if (!o || !o->property("enabled").toBool()) return false;
        QMetaObject::invokeMethod(o, "click");
        d.spin(80);
        return true;
    }
    // Scrolls page `page` so that `name` is at its top (for a screenshot)
    void scrollTo(const QString &page, const QString &name) const {
        auto *flick = qobject_cast<QQuickItem *>(get(page));
        auto *item = qobject_cast<QQuickItem *>(get(name));
        auto *content = flick ? flick->property("contentItem").value<QQuickItem *>() : nullptr;
        if (!flick || !item || !content) return;
        const qreal top = item->mapToItem(content, QPointF(0, 0)).y();
        const qreal most = std::max<qreal>(0, flick->property("contentHeight").toReal() - flick->height());
        flick->setProperty("contentY", std::clamp<qreal>(top - 8, 0, most));
        d.spin(120);
    }
    // A row's popup: choose the option whose value is `value`, as a person does
    bool choose(const QString &name, int value) const {
        QObject *combo = get(name);
        if (!combo) return false;
        const QVariantList options = combo->property("options").toList();
        for (int i = 0; i < options.size(); ++i) {
            if (options.at(i).toMap().value(QStringLiteral("value")).toInt() == value) {
                QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, i));
                d.spin(80);
                return true;
            }
        }
        return false;
    }
};

QVariantMap qp12Row(const QVariantMap &rec, const QString &id) {
    for (const QVariant &r : rec.value(QStringLiteral("rows")).toList())
        if (r.toMap().value(QStringLiteral("id")).toString() == id) return r.toMap();
    return {};
}

// The first option of a pick row that is offered and not the current value
int qp12OtherOption(const QVariantMap &row) {
    for (const QVariant &o : row.value(QStringLiteral("options")).toList()) {
        const QVariantMap m = o.toMap();
        if (!m.value(QStringLiteral("disabled")).toBool() && m.value(QStringLiteral("value")) != row.value(QStringLiteral("value")))
            return m.value(QStringLiteral("value")).toInt();
    }
    return -1;
}

void qp12Open(SmokeDriver &d, SettingsController *s, const char *page) {
    s->open(QString::fromLatin1(page));
    d.spin(300);
    d.waitIdle();
    d.spin(200);
}

void surfacesSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    DeviceModel *device = d.app().device;
    auto *cs = Registrar::instance<CsController>();
    auto *s = Registrar::instance<SettingsController>();
    auto *hw = Registrar::instance<HardwareController>();
    d.check(QStringLiteral("qp12: the controller exists"), cs && s && hw);
    if (!cs || !s || !hw) return;
    d.check(QStringLiteral("qp12: the demo's control surfaces are read"), d.waitFor(5000, [&]() { return cs->loaded(); }));
    d.waitIdle();
    const auto settle = [&](const std::function<bool()> &done, int ms = 3000) {
        const bool ok = d.waitFor(ms, done);
        d.waitIdle();
        d.spin(120);
        return ok;
    };
    const auto rec = [&](const QString &key) { return cs->record(key); };

    // ---- the pages, in the Mac's Control group, and the search ---------------------
    QStringList ids;
    for (const QVariant &row : s->sidebarRows(QString()))
        if (row.toMap().contains(QStringLiteral("id"))) ids << row.toMap().value(QStringLiteral("id")).toString();
    const auto at = [&](const char *id) { return ids.indexOf(QString::fromLatin1(id)); };
    d.check(QStringLiteral("qp12: Control lists Control Surfaces, Control Interfaces, Channel Groups, Macros, Auxiliary Outputs"),
            at("surfaces") >= 0 && at("groups") > at("surfaces") && at("macros") == at("groups") + 1 && at("aux") == at("macros") + 1
                && (at("interfaces") < 0 || (at("interfaces") > at("surfaces") && at("interfaces") < at("groups"))));
    const auto found = [&](const char *query) {
        QStringList out;
        for (const QVariant &row : s->sidebarRows(QString::fromLatin1(query)))
            if (row.toMap().contains(QStringLiteral("id"))) out << row.toMap().value(QStringLiteral("id")).toString();
        return out;
    };
    d.check(QStringLiteral("qp12: \"learn\" leaves Control Surfaces"), found("learn").contains(QStringLiteral("surfaces")));
    d.check(QStringLiteral("qp12: \"relay\" leaves Auxiliary Outputs"), found("relay").contains(QStringLiteral("aux")));
    d.check(QStringLiteral("qp12: \"wait before\" leaves Macros"), found("wait before").contains(QStringLiteral("macros")));

    shell->trigger(QStringLiteral("edit.settings"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("qp12: the Settings window opens"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    Qp12Ui ui{ d, w };

    // ---- Control Surfaces: the cards ----------------------------------------------
    qp12Open(d, s, "surfaces");
    d.check(QStringLiteral("qp12: Control Surfaces opens"), ui.shown(QStringLiteral("SurfacesPage")));
    const QStringList controls = cs->keys(QStringLiteral("controls"));
    d.check(QStringLiteral("qp12: the demo's controls are listed, aux outputs apart"),
            controls.contains(QStringLiteral("binding/0")) && !controls.contains(QStringLiteral("binding/10"))
                && cs->keys(QStringLiteral("aux")).contains(QStringLiteral("binding/10")));
    d.check(QStringLiteral("qp12: a card shows its name and what it does"),
            settle([&]() { return ui.text(QStringLiteral("Cs.binding.0.name")) == QLatin1String("Volume Knob")
                                  && !ui.text(QStringLiteral("Cs.binding.0.summary")).isEmpty(); }));
    d.check(QStringLiteral("qp12: a running control reads Active"), ui.text(QStringLiteral("Cs.binding.0.pill.text")) == QLatin1String("Active"));
    d.check(QStringLiteral("qp12: cards start collapsed"), !ui.shown(QStringLiteral("Cs.binding.1.rows")));
    ui.click(QStringLiteral("Cs.binding.1.disclose"));
    d.check(QStringLiteral("qp12: a card opens on its editor, the rows the core describes"),
            settle([&]() { return ui.shown(QStringLiteral("Cs.binding.1.rows.row.action.combo")); }));
    d.check(QStringLiteral("qp12: a pin someone holds is named and not offered"), [&]() {
        for (const QVariant &o : qp12Row(rec(QStringLiteral("binding/1")), QStringLiteral("gpio0")).value(QStringLiteral("options")).toList())
            if (o.toMap().value(QStringLiteral("disabled")).toBool() && !o.toMap().value(QStringLiteral("owner")).toString().isEmpty()) return true;
        return false;
    }());
    d.grab(w, QStringLiteral("settings-surfaces.png"));

    // ---- an edit waits for Apply; Revert drops it ----------------------------------
    const QVariantMap action = qp12Row(rec(QStringLiteral("binding/1")), QStringLiteral("action"));
    const int actionWas = action.value(QStringLiteral("value")).toInt();
    const int actionNew = qp12OtherOption(action);
    int setters = device->settersRun();
    d.check(QStringLiteral("qp12: a row's choice changes the draft"),
            ui.choose(QStringLiteral("Cs.binding.1.rows.row.action.combo"), actionNew)
                && settle([&]() { return qp12Row(rec(QStringLiteral("binding/1")), QStringLiteral("action")).value(QStringLiteral("value")).toInt() == actionNew; }));
    d.check(QStringLiteral("qp12: and the card reads Pending"), ui.text(QStringLiteral("Cs.binding.1.pill.text")) == QLatin1String("Pending"));
    d.check(QStringLiteral("qp12: and nothing is sent"), device->settersRun() == setters && !cs->dirty());
    ui.click(QStringLiteral("Cs.binding.1.applyRow.revert"));
    d.check(QStringLiteral("qp12: Revert drops the edit"),
            settle([&]() { return !rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool()
                                  && qp12Row(rec(QStringLiteral("binding/1")), QStringLiteral("action")).value(QStringLiteral("value")).toInt() == actionWas; }));

    // ---- Apply makes it live, the save bar offers to keep it, its Revert puts it back
    ui.choose(QStringLiteral("Cs.binding.1.rows.row.action.combo"), actionNew);
    settle([&]() { return rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool(); });
    d.check(QStringLiteral("qp12: Apply sends the binding"),
            ui.click(QStringLiteral("Cs.binding.1.applyRow.apply"))
                && settle([&]() { return !rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool() && cs->dirty(); }));
    d.check(QStringLiteral("qp12: the device holds the new action"),
            qp12Row(rec(QStringLiteral("binding/1")), QStringLiteral("action")).value(QStringLiteral("value")).toInt() == actionNew);
    d.check(QStringLiteral("qp12: the save bar shows, saying the controls are live"),
            settle([&]() { return ui.shown(QStringLiteral("SaveBar")); })
                && ui.text(QStringLiteral("SaveBar.subtitle")).startsWith(QLatin1String("Your controls are live now")));
    d.grab(w, QStringLiteral("settings-surfaces-applied.png"));
    ui.click(QStringLiteral("SaveBar.revert"));
    d.check(QStringLiteral("qp12: the bar's Revert reloads what is stored"),
            settle([&]() { return !cs->dirty() && qp12Row(rec(QStringLiteral("binding/1")), QStringLiteral("action")).value(QStringLiteral("value")).toInt() == actionWas; }, 5000));
    d.check(QStringLiteral("qp12: and the bar goes"), settle([&]() { return !ui.shown(QStringLiteral("SaveBar")); }));

    // ---- a rename is held, then applied ---------------------------------------------
    cs->setName(QStringLiteral("binding/1"), QStringLiteral("Mute Key"));
    d.check(QStringLiteral("qp12: a rename is held"), rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool());
    cs->apply(QStringLiteral("binding/1"));
    d.check(QStringLiteral("qp12: Apply names the control on the device"),
            settle([&]() { return !rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool()
                                  && rec(QStringLiteral("binding/1")).value(QStringLiteral("name")).toString() == QLatin1String("Mute Key"); }));
    d.check(QStringLiteral("qp12: a named card restates its type in the summary"),
            rec(QStringLiteral("binding/1")).value(QStringLiteral("summary")).toString().startsWith(QLatin1String("Push Button - ")));

    // ---- a refused apply keeps the draft and says why ---------------------------------
    d.app().core->worker()->post([](FfiCore *c) { dspi_cs_demo_busy(c, true); });
    d.waitIdle();
    cs->edit(QStringLiteral("binding/1"), QStringLiteral("action"), actionNew);
    settle([&]() { return rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool(); });
    cs->apply(QStringLiteral("binding/1"));
    d.check(QStringLiteral("qp12: a refused apply shows the device's reason"),
            settle([&]() { const QVariantMap r = rec(QStringLiteral("binding/1"));
                           return !r.value(QStringLiteral("applying")).toBool() && r.value(QStringLiteral("isError")).toBool()
                                  && !r.value(QStringLiteral("message")).toString().isEmpty(); }, 5000));
    d.check(QStringLiteral("qp12: and keeps the draft"), rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool());
    d.check(QStringLiteral("qp12: the reason shows on the card"),
            ui.text(QStringLiteral("Cs.binding.1.applyRow.text")) == rec(QStringLiteral("binding/1")).value(QStringLiteral("message")).toString());
    d.app().core->worker()->post([](FfiCore *c) { dspi_cs_demo_busy(c, false); });
    d.waitIdle();
    cs->revertRecord(QStringLiteral("binding/1"));
    settle([&]() { return !rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool(); });

    // ---- adding a control applies it at once; removing clears the slot ------------------
    const int claimsBefore = int(s->pinClaims().size());
    const QString added = cs->addControl(CsController::Button);
    d.check(QStringLiteral("qp12: Add Control takes a free slot"), !added.isEmpty());
    d.check(QStringLiteral("qp12: the new control is live at once, its card open"),
            settle([&]() { return rec(added).value(QStringLiteral("liveConfigured")).toBool() && !rec(added).value(QStringLiteral("dirty")).toBool(); }, 5000)
                && settle([&]() { return ui.shown(QStringLiteral("Cs.") + QString(added).replace(QLatin1Char('/'), QLatin1Char('.')) + QStringLiteral(".rows")); }));
    const int newPin = qp12Row(rec(added), QStringLiteral("gpio0")).value(QStringLiteral("value")).toInt();
    d.check(QStringLiteral("qp12: the Overview's map holds its pin"), settle([&]() {
        for (const QVariant &c : s->pinClaims())
            if (c.toMap().value(QStringLiteral("pin")).toInt() == newPin) return int(s->pinClaims().size()) > claimsBefore;
        return false;
    }));
    d.check(QStringLiteral("qp12: the hardware pin pickers leave it out"), [&]() {
        for (const QVariant &c : hw->pinChoices(0, 0, -1, false))
            if (c.toMap().value(QStringLiteral("pin")).toInt() == newPin) return false;
        return true;
    }());
    cs->remove(added);
    d.check(QStringLiteral("qp12: Remove clears the slot on the device"),
            settle([&]() { return !cs->keys(QStringLiteral("controls")).contains(added); }, 5000));

    // ---- the IR receiver's remote buttons, the learn ----------------------------------
    ui.click(QStringLiteral("Cs.binding.3.disclose"));
    settle([&]() { return ui.shown(QStringLiteral("Cs.binding.3.addRemote")); });
    d.check(QStringLiteral("qp12: the receiver lists its remote buttons"),
            ui.shown(QStringLiteral("Cs.ir.0")) && ui.text(QStringLiteral("Cs.ir.0.code")).contains(QLatin1String("0x")));
    const QStringList remotesBefore = cs->keys(QStringLiteral("remotes"));
    ui.click(QStringLiteral("Cs.binding.3.addRemote"));
    d.check(QStringLiteral("qp12: Add Remote Button adds one, not learned"),
            settle([&]() { return cs->keys(QStringLiteral("remotes")).size() == remotesBefore.size() + 1; }));
    QString sub;
    for (const QString &k : cs->keys(QStringLiteral("remotes")))
        if (!remotesBefore.contains(k)) sub = k;
    const QString subName = QStringLiteral("Cs.") + QString(sub).replace(QLatin1Char('/'), QLatin1Char('.'));
    d.check(QStringLiteral("qp12: it reads Not learned"), ui.text(subName + QStringLiteral(".code")) == QLatin1String("Not learned"));
    ui.click(subName + QStringLiteral(".learn"));
    d.check(QStringLiteral("qp12: Learn arms the receiver and says what to do"),
            settle([&]() { return cs->learningSub() >= 0 && ui.shown(subName + QStringLiteral(".learnMessage")); }));
    ui.scrollTo(QStringLiteral("SurfacesPage"), QStringLiteral("Cs.binding.3"));
    d.grab(w, QStringLiteral("settings-surfaces-learn.png"));
    d.app().core->worker()->post([](FfiCore *c) { dspi_cs_demo_learn(c, 1, 0x00FF12EDu); });
    d.check(QStringLiteral("qp12: a pressed remote button is learned"),
            settle([&]() { return cs->learningSub() < 0 && rec(sub).value(QStringLiteral("learned")).toBool(); }, 5000));
    d.check(QStringLiteral("qp12: its code shows, with Apply to keep it"),
            ui.text(subName + QStringLiteral(".code")) == QLatin1String("NEC 0x00FF12ED")
                && rec(sub).value(QStringLiteral("message")).toString().contains(QLatin1String("Apply to keep it")));
    d.check(QStringLiteral("qp12: the receiver reads Pending for it"), rec(QStringLiteral("binding/3")).value(QStringLiteral("dirty")).toBool());
    ui.click(QStringLiteral("Cs.binding.3.applyRow.apply"));
    d.check(QStringLiteral("qp12: the receiver's Apply stores the remote button"),
            settle([&]() { return rec(sub).value(QStringLiteral("liveLearned")).toBool() && !rec(QStringLiteral("binding/3")).value(QStringLiteral("dirty")).toBool(); }, 5000));
    ui.click(subName + QStringLiteral(".learn"));
    settle([&]() { return cs->learningSub() >= 0; });
    ui.click(subName + QStringLiteral(".learn"));
    d.check(QStringLiteral("qp12: Cancel ends a learn"),
            settle([&]() { return cs->learningSub() < 0 && rec(sub).value(QStringLiteral("message")).toString() == QLatin1String("Learn cancelled."); }));
    d.check(QStringLiteral("qp12: and the busy claim goes"), settle([&]() { return !cs->busy(); }));
    cs->remove(sub);
    d.check(QStringLiteral("qp12: a remote button is removed"),
            settle([&]() { return cs->keys(QStringLiteral("remotes")) == remotesBefore; }, 5000));
    ui.click(QStringLiteral("Cs.binding.3.disclose"));

    // ---- the display: its settings and pages apply at once --------------------------
    ui.click(QStringLiteral("Cs.binding.4.disclose"));
    d.check(QStringLiteral("qp12: the display card shows the panel's state and its settings"),
            settle([&]() { return ui.shown(QStringLiteral("Cs.display.state")) && !cs->display().value(QStringLiteral("rows")).toList().isEmpty(); }));
    const int pagesBefore = int(cs->display().value(QStringLiteral("pages")).toList().size());
    d.check(QStringLiteral("qp12: Add Page adds a page on the device"),
            ui.click(QStringLiteral("Cs.display.addPage"))
                && settle([&]() { return cs->display().value(QStringLiteral("pages")).toList().size() == pagesBefore + 1; }, 5000));
    ui.scrollTo(QStringLiteral("SurfacesPage"), QStringLiteral("Cs.binding.4"));
    d.grab(w, QStringLiteral("settings-surfaces-display.png"));
    const QVariantList pages = cs->display().value(QStringLiteral("pages")).toList();
    const int lastPage = pages.isEmpty() ? -1 : pages.last().toMap().value(QStringLiteral("index")).toInt();
    cs->removePage(lastPage);
    d.check(QStringLiteral("qp12: and removes it"),
            settle([&]() { return cs->display().value(QStringLiteral("pages")).toList().size() == pagesBefore; }, 5000));
    ui.click(QStringLiteral("Cs.binding.4.disclose"));

    // ---- Auxiliary Outputs: the switch and level at once, never to flash ---------------
    qp12Open(d, s, "aux");
    d.check(QStringLiteral("qp12: Auxiliary Outputs lists the outputs"), ui.shown(QStringLiteral("AuxPage")) && ui.shown(QStringLiteral("Cs.binding.10")));
    ui.click(QStringLiteral("Cs.binding.11.disclose"));
    settle([&]() { return ui.shown(QStringLiteral("Cs.binding.11.auxLevel")); });
    const bool onWas = rec(QStringLiteral("binding/10")).value(QStringLiteral("auxOn")).toBool();
    cs->setAuxState(10, !onWas);
    d.check(QStringLiteral("qp12: the Output switch switches the pin now"),
            settle([&]() { return rec(QStringLiteral("binding/10")).value(QStringLiteral("auxOn")).toBool() == !onWas; }));
    const double levelWas = rec(QStringLiteral("binding/11")).value(QStringLiteral("auxLevel")).toDouble();
    cs->setAuxLevel(11, 30);
    d.check(QStringLiteral("qp12: the Level sets the dimmable output"),
            settle([&]() { return std::abs(rec(QStringLiteral("binding/11")).value(QStringLiteral("auxLevel")).toDouble() - 30) < 0.6; }));
    d.check(QStringLiteral("qp12: the aux card names what drives it"),
            !qp12Row(rec(QStringLiteral("binding/10")), QStringLiteral("drivenBy")).isEmpty());
    d.grab(w, QStringLiteral("settings-aux.png"));
    cs->setAuxState(10, onWas);
    cs->setAuxLevel(11, levelWas);
    settle([&]() { return rec(QStringLiteral("binding/10")).value(QStringLiteral("auxOn")).toBool() == onWas; });

    // ---- Channel Groups: a group waits for members, then for Apply ----------------------
    qp12Open(d, s, "groups");
    d.check(QStringLiteral("qp12: Channel Groups lists the demo's group"), ui.shown(QStringLiteral("GroupsPage")) && ui.shown(QStringLiteral("Cs.group.0")));
    const QString group = cs->addGroup();
    const QString groupName = QStringLiteral("Cs.") + QString(group).replace(QLatin1Char('/'), QLatin1Char('.'));
    d.check(QStringLiteral("qp12: Add Group opens a new card, named and not applied"),
            !group.isEmpty() && settle([&]() { return ui.shown(groupName + QStringLiteral(".rows")); })
                && ui.text(groupName + QStringLiteral(".name")).startsWith(QLatin1String("Group ")));
    d.check(QStringLiteral("qp12: with no member it asks for one, Apply off"),
            ui.text(groupName + QStringLiteral(".applyRow.text")) == QLatin1String("Pick at least one channel.")
                && !ui.get(groupName + QStringLiteral(".applyRow.apply"))->property("enabled").toBool());
    QString member;
    for (const QVariant &r : rec(group).value(QStringLiteral("rows")).toList())
        if (r.toMap().value(QStringLiteral("id")).toString().startsWith(QLatin1String("member."))) {
            member = r.toMap().value(QStringLiteral("id")).toString();
            break;
        }
    cs->edit(group, member, true);
    d.check(QStringLiteral("qp12: a member makes it one the device takes"),
            settle([&]() { return rec(group).value(QStringLiteral("configured")).toBool(); }));
    d.grab(w, QStringLiteral("settings-groups.png"));
    ui.click(groupName + QStringLiteral(".applyRow.apply"));
    d.check(QStringLiteral("qp12: Apply stores the group"),
            settle([&]() { return rec(group).value(QStringLiteral("liveConfigured")).toBool() && !rec(group).value(QStringLiteral("dirty")).toBool(); }, 5000));
    cs->remove(group);
    d.check(QStringLiteral("qp12: a group is removed"), settle([&]() { return !cs->keys(QStringLiteral("groups")).contains(group); }, 5000));

    // ---- Macros: steps wait for Apply; Run plays the stored one -------------------------
    qp12Open(d, s, "macros");
    d.check(QStringLiteral("qp12: Macros lists the demo's macro"), ui.shown(QStringLiteral("MacrosPage")) && ui.shown(QStringLiteral("Cs.macro.0")));
    const QString macro = cs->addMacro();
    const QString macroName = QStringLiteral("Cs.") + QString(macro).replace(QLatin1Char('/'), QLatin1Char('.'));
    d.check(QStringLiteral("qp12: Add Macro opens a new card with no steps"),
            !macro.isEmpty() && settle([&]() { return ui.shown(macroName + QStringLiteral(".addStep")); })
                && ui.text(macroName + QStringLiteral(".summary")) == QLatin1String("No steps yet."));
    d.check(QStringLiteral("qp12: Run waits until the macro is stored"), !ui.get(macroName + QStringLiteral(".run"))->property("enabled").toBool());
    ui.click(macroName + QStringLiteral(".addStep"));
    d.check(QStringLiteral("qp12: Add Step adds a step with its rows"),
            settle([&]() { return rec(macro).value(QStringLiteral("stepCount")).toInt() == 1
                                  && ui.shown(macroName + QStringLiteral(".step.0.row.preDelay.spin")); }));
    cs->editStep(macro, 0, QStringLiteral("preDelay"), 0.5);
    d.check(QStringLiteral("qp12: a step's wait shows in the summary"),
            settle([&]() { return rec(macro).value(QStringLiteral("summary")).toString().contains(QLatin1String("total")); }));
    d.grab(w, QStringLiteral("settings-macros.png"));
    ui.click(macroName + QStringLiteral(".applyRow.apply"));
    d.check(QStringLiteral("qp12: Apply stores the macro"),
            settle([&]() { return rec(macro).value(QStringLiteral("liveSteps")).toInt() == 1 && !rec(macro).value(QStringLiteral("dirty")).toBool(); }, 5000));
    d.check(QStringLiteral("qp12: Run is offered once it is stored"),
            settle([&]() { return ui.get(macroName + QStringLiteral(".run"))->property("enabled").toBool(); }));
    ui.click(macroName + QStringLiteral(".run"));
    d.check(QStringLiteral("qp12: Run fires it on the device"), settle([&]() { return !cs->busy(); }, 3000)
                && rec(macro).value(QStringLiteral("message")).toString().isEmpty());
    settle([&]() { return !rec(macro).value(QStringLiteral("running")).toBool(); }, 3000);
    cs->remove(macro);
    d.check(QStringLiteral("qp12: a macro is removed"), settle([&]() { return !cs->keys(QStringLiteral("macros")).contains(macro); }, 5000));

    // ---- the save bar's Save keeps the live configuration ------------------------------
    d.check(QStringLiteral("qp12: the changes since are live and unsaved"), settle([&]() { return cs->dirty() && ui.shown(QStringLiteral("SaveBar")); }));
    ui.click(QStringLiteral("SaveBar.save"));
    d.check(QStringLiteral("qp12: Save writes them to flash"), settle([&]() { return !cs->dirty(); }, 5000));
    d.check(QStringLiteral("qp12: and the bar goes"), settle([&]() { return !ui.shown(QStringLiteral("SaveBar")); }));

    // The demo's control back to its first name
    cs->setName(QStringLiteral("binding/1"), QString());
    cs->apply(QStringLiteral("binding/1"));
    settle([&]() { return !rec(QStringLiteral("binding/1")).value(QStringLiteral("dirty")).toBool(); });
    if (cs->dirty()) {
        cs->save({});
        settle([&]() { return !cs->dirty(); }, 5000);
    }
    d.check(QStringLiteral("qp12: the demo's names are back"), rec(QStringLiteral("binding/1")).value(QStringLiteral("name")).toString().isEmpty());

    // ---- read-only firmware: nothing is offered and nothing is sent ----------------------
    qp12Open(d, s, "surfaces");
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_wire(c, 17); });
    device->refresh();
    d.check(QStringLiteral("qp12: the V17 recording opens read-only"), d.waitFor(5000, [&]() { return device->readOnly(); }));
    settle([&]() { return !cs->writable(); });
    d.check(QStringLiteral("qp12: read-only: the control surfaces are not writable"), !cs->writable());
    const int roSetters = device->settersRun();
    cs->addControl(CsController::Button);
    cs->apply(QStringLiteral("binding/0"));
    cs->setAuxState(10, true);
    d.waitIdle();
    d.check(QStringLiteral("qp12: read-only sends nothing"), device->settersRun() == roSetters);
    d.check(QStringLiteral("qp12: firmware without control surfaces lists no Control Surfaces page"),
            settle([&]() { return cs->supported() || !s->isShown(QStringLiteral("surfaces")); }, 5000));
    device->openDemo();
    d.check(QStringLiteral("qp12: the demo reopens with its control surfaces"),
            d.waitFor(5000, [&]() { return !device->readOnly() && device->connected(); })
                && settle([&]() { return cs->loaded() && cs->writable() && s->isShown(QStringLiteral("surfaces")); }, 5000));
    w->close();
    d.spin(100);
}

const SmokeDriver::Section surfaces("qp12-surfaces", 47, &surfacesSection);

} // namespace
