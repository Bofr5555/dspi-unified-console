#include "DeviceLinkController.h"

#include "Confirm.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "LayoutController.h"
#include "NoticeCenter.h"
#include "SessionModel.h"
#include "SidebarController.h"

#include <QJSEngine>
#include <cmath>
#include <cstring>
#include <memory>

static const Registrar::Controller<DeviceLinkController> deviceLinkController;

namespace {
// What a fixed-size name field of the device says
QString fieldText(const uint8_t *field, size_t size) {
    const char *s = reinterpret_cast<const char *>(field);
    return QString::fromUtf8(s, qsizetype(strnlen(s, size))).trimmed();
}

bool occupied(const DspState *s, int slot) { return s && slot >= 0 && slot < MAX_PRESETS && (s->preset_occupied >> slot) & 1u; }

QString presetName(const DspState *s, int slot) {
    const QString name = fieldText(s->preset_names[slot], CHANNEL_NAME_LEN);
    return name.isEmpty() ? DeviceLinkController::tr("Preset %1").arg(slot + 1) : name;
}
} // namespace

DeviceLinkController::DeviceLinkController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // The shown device is the one the others keep their offsets from
    connect(ctx.registry, &DeviceRegistry::activeChanged, this, [this] {
        m_offsets.clear();
        emit changed();
    });
    connect(ctx.registry, &DeviceRegistry::devicesChanged, this, [this] {
        // A device that went is forgotten; with fewer than two the link is off
        if (m_linked && !available()) setLinked(false);
        emit changed();
    });
    if (auto *layout = Registrar::instance<LayoutController>())
        connect(layout, &LayoutController::layoutChanged, this, [this] {
            if (m_linked && !available()) setLinked(false);
            emit changed();
        });
}

DeviceLinkController *DeviceLinkController::create(QQmlEngine *, QJSEngine *) {
    DeviceLinkController *self = Registrar::instance<DeviceLinkController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QList<DeviceSession *> DeviceLinkController::targets() const {
    QList<DeviceSession *> out;
    if (!m_ctx.registry) return out;
    DeviceSession *shown = m_ctx.activeSession();
    for (DeviceSession *s : m_ctx.registry->sessions()) {
        if (!s->model()->connected() || s->model()->readOnly()) continue;
        if (s == shown) out.prepend(s);
        else out.append(s);
    }
    return out;
}

bool DeviceLinkController::available() const {
    auto *layout = Registrar::instance<LayoutController>();
    return (!layout || layout->modern()) && targets().size() >= 2 && targets().first() == m_ctx.activeSession();
}

void DeviceLinkController::setLinked(bool on) {
    if (on && !available()) on = false;
    if (on == m_linked) return;
    m_linked = on;
    m_offsets.clear();
    emit changed();
}

QStringList DeviceLinkController::names() const {
    QStringList out;
    for (DeviceSession *s : targets()) out << s->title();
    return out;
}

QString DeviceLinkController::help() const {
    const QStringList all = names();
    if (all.size() < 2) return {};
    //: %1 lists the open devices, "Fronts, Centre, Surrounds"
    return linked() ? tr("All devices linked: %1. The User volume moves them all and keeps their differences, Mute mutes them all, a "
                         "preset is recalled on all after one question. Click to unlink.")
                          .arg(all.join(QStringLiteral(", ")))
                    : tr("Link all devices (%1): the User volume, Mute and the preset act on every one.").arg(all.join(QStringLiteral(", ")));
}

// ---- Volume and mute ---------------------------------------------------------------

void DeviceLinkController::setUserVolume(double db) {
    const double v = qBound(SidebarController::kUserMinDb, db, 0.0);
    const QList<DeviceSession *> all = targets();
    if (all.isEmpty()) return;
    DeviceSession *shown = all.first();
    const DspState *here = shown->model()->state();
    if (linked() && here) {
        for (DeviceSession *s : all.mid(1)) {
            const DspState *st = s->model()->state();
            if (!st) continue;
            // Taken once, while the link is on and this device is shown
            if (!m_offsets.contains(s->id())) m_offsets.insert(s->id(), double(st->user_volume_db) - double(here->user_volume_db));
            const float to = float(qBound(SidebarController::kUserMinDb, v + m_offsets.value(s->id()), 0.0));
            if (std::abs(to - st->user_volume_db) < 0.005f) continue;
            s->model()->write(QStringLiteral("user volume (all devices)"), [to](FfiCore *c) { return dspi_set_user_volume(c, to); });
        }
    }
    // The shown device as the dock's own move
    if (auto *sidebar = Registrar::instance<SidebarController>()) sidebar->setUserVolume(v);
}

void DeviceLinkController::setMute(bool on) {
    const QList<DeviceSession *> all = targets();
    for (int i = 0; i < all.size(); ++i) {
        if (i > 0 && !linked()) break;
        const DspState *st = all.at(i)->model()->state();
        if (st && st->user_mute == on) continue;
        all.at(i)->model()->write(i == 0 ? QStringLiteral("user mute") : QStringLiteral("user mute (all devices)"),
                                  [on](FfiCore *c) { return dspi_set_user_mute(c, on); });
    }
}

// ---- A preset on every device ------------------------------------------------------

bool DeviceLinkController::takePresetSwitch(int slot) {
    if (!linked() || slot < 0 || slot >= MAX_PRESETS) return false;
    if (m_askedSlot >= 0) return true;   // one question at a time
    QStringList lines;
    int recalls = 0;
    for (DeviceSession *s : targets()) {
        const DspState *st = s->model()->state();
        if (!occupied(st, slot)) {
            //: a line of the question; %1 is a device's name
            lines << tr("• %1: no preset %2 there, it stays as it is").arg(s->title()).arg(slot + 1);
            continue;
        }
        ++recalls;
        const int unsaved = m_ctx.registry->unsavedChanges(s).size();
        //: a line of the question; %1 is a device's name, %2 the preset's name
        QString line = tr("• %1: “%2”").arg(s->title(), presetName(st, slot));
        if (unsaved == 1) line += tr(" (its 1 unsaved change is dropped)");
        else if (unsaved > 1) line += tr(" (its %1 unsaved changes are dropped)").arg(unsaved);
        lines << line;
    }
    if (recalls == 0) {
        if (auto *notices = Registrar::instance<NoticeCenter>())
            notices->post(tr("No linked device has a preset %1.").arg(slot + 1), QStringLiteral("warning"));
        return true;
    }
    m_askedSlot = slot;
    emit recallQuestion(tr("Recall preset %1 on all devices?").arg(slot + 1), lines.join(QLatin1Char('\n')));
    return true;
}

void DeviceLinkController::answerRecall(const QString &answer) {
    const int slot = m_askedSlot;
    if (slot < 0) return;
    if (answer != QLatin1String("accept")) {
        m_askedSlot = -1;
        return;
    }
    m_recall.clear();
    for (DeviceSession *s : targets())
        if (occupied(s->model()->state(), slot)) m_recall << s;
    m_recalled = 0;
    m_recallFailed.clear();
    recallNext();
}

void DeviceLinkController::recallNext() {
    const int slot = m_askedSlot;
    while (!m_recall.isEmpty()) {
        const QPointer<DeviceSession> s = m_recall.takeFirst();
        if (!s || !s->model()->connected()) continue;
        auto status = std::make_shared<uint8_t>(0xFF);
        s->model()->write(
            QStringLiteral("preset load (all devices)"),
            [slot, status](FfiCore *c) {
                *status = Confirm::loadPreset(c, uint8_t(slot));
                return *status == PRESET_OK;
            },
            [this, s](bool ok) {
                if (ok) ++m_recalled;
                else if (s) m_recallFailed << s->title();
                recallNext();
            });
        return;
    }
    m_askedSlot = -1;
    m_offsets.clear();   // a preset may hold another volume
    if (auto *notices = Registrar::instance<NoticeCenter>()) {
        if (!m_recallFailed.isEmpty())
            //: %1 is the preset's number, %2 the devices it did not load on
            notices->post(tr("Preset %1 was not recalled on %2.").arg(slot + 1).arg(m_recallFailed.join(QStringLiteral(", "))),
                          QStringLiteral("error"));
        else
            notices->post(m_recalled == 1 ? tr("Preset %1 recalled on 1 device.").arg(slot + 1)
                                          : tr("Preset %1 recalled on %2 devices.").arg(slot + 1).arg(m_recalled));
    }
    emit recalled(slot, m_recalled, m_recallFailed);
    emit changed();
}
