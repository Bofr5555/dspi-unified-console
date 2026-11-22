// -- WP QP16 -- Read-only mode and the delay refusal (ReadOnlyController.h)
#include "ReadOnlyController.h"

#include "AppIdentity.h"
#include "DeviceModel.h"
#include "FirmwareController.h"
#include "Shell.h"

#include <QJSEngine>

static const Registrar::Controller<ReadOnlyController> readOnlyController;
static const Registrar::Banner readOnlyBanner("ReadOnlyBanner", 11, Registrar::Blocking);
static const Registrar::Banner delayRefusalBanner("DelayRefusalBanner", 30, Registrar::Blocking);

ReadOnlyController::ReadOnlyController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] {
        emit changed();
        // Another device (or none): its refusal is not this one's
        const quint64 generation = m_ctx.device->snapshot().generation;
        if (generation != m_generation) {
            m_generation = generation;
            setDelayRefusal(QString());
        }
    });
    connect(ctx.device, &DeviceModel::writeFinished, this,
            [this](const QString &what, bool ok, const QString &code, const QString &message) {
                // The output delays' writes (ChannelController, the matrix)
                if (!what.startsWith(QLatin1String("output delay "))) return;
                if (ok) setDelayRefusal(QString());
                else if (code == QLatin1String("invalid") && !message.isEmpty()) {
                    // The core's sentence, without the name of its error kind
                    static const QLatin1String kind("invalid argument: ");
                    setDelayRefusal(message.startsWith(kind) ? message.mid(kind.size()) : message);
                }
            });
}

ReadOnlyController *ReadOnlyController::create(QQmlEngine *, QJSEngine *) {
    ReadOnlyController *self = Registrar::instance<ReadOnlyController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

bool ReadOnlyController::active() const { return m_ctx.device->readOnly(); }

bool ReadOnlyController::newer() const { return m_ctx.device->compat() == COMPAT_FIRMWARE_TOO_NEW; }

QString ReadOnlyController::text() const {
    if (!active()) return QString();
    const FirmwareController *firmware = Registrar::instance<FirmwareController>();
    const QString version = firmware && !firmware->runningVersion().isEmpty() ? firmware->runningVersion()
                                                                            : m_ctx.device->firmwareVersion();
    const QString expected = firmware ? firmware->expectedVersion() : QString();
    if (newer())
        return tr("This device runs firmware %1, which is newer than %2 understands. Its settings are shown read-only, "
                  "and some of its features may not show. Update %2 to change them.")
            .arg(version, AppIdentity::displayName());
    return tr("This device runs firmware %1, which is too old for %2. Its settings are shown read-only, and features "
              "it lacks are hidden. Update it to firmware %3 to change them.")
        .arg(version, AppIdentity::displayName(), expected);
}

QString ReadOnlyController::actionText() const { return newer() ? tr("Details…") : tr("Update…"); }

// No firmware window on STM32H723
bool ReadOnlyController::actionShown() const {
    return active() && m_ctx.device->platformName() != QLatin1String("STM32H723");
}

void ReadOnlyController::openDetails() {
    m_ctx.shell->trigger(QStringLiteral("device.firmwareUpdate"));
}

void ReadOnlyController::dismissDelayRefusal() { setDelayRefusal(QString()); }

void ReadOnlyController::setDelayRefusal(const QString &text) {
    // The core's sentence as it is (behaviour.md §15)
    if (text == m_delayRefusal) return;
    m_delayRefusal = text;
    emit delayRefusalChanged();
}
