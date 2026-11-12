// -- WP QP17 -- Getting Started, What's New, the About licence block and the
// Help menu's rows (OnboardingController.h)
#include "OnboardingController.h"

#include "AppIdentity.h"
#include "AppUpdater.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "FilesController.h"
#include "SettingsController.h"
#include "Shell.h"
#include "Snapshot.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlEngine>
#include <QSettings>
#include <QStandardPaths>
#include <algorithm>

// The firmware the build ships, from the names of its bundled images
// (cmake/wp/qp17.cmake)
#ifndef DSPI_BUNDLED_FIRMWARE
#define DSPI_BUNDLED_FIRMWARE ""
#endif

namespace {

const Registrar::Controller<OnboardingController> onboardingController;
const Registrar::Window whatsNewWindow("help.whatsNew", "WhatsNewWindow");
const SettingsRegistry::Section licenceSection{ "about", 50, "AboutLicenceSection" };
const SettingsRegistry::Section developerSection{ "advanced", 50, "OnboardingDeveloperSection" };

// The keys behaviour.md §21.2 names, so a user's state carries over
const char *const kSetupDoneKey = "onboarding/setupDone";
const char *const kWhatsNewKey = "onboarding/whatsNewShown";

using Version = AppUpdater::Version;

QJsonDocument coreJson(uint32_t (*fill)(char *, uint32_t), uint32_t first) {
    return QJsonDocument::fromJson(readSized([fill](char *buf, uint32_t len) { return fill(buf, len); }, first));
}

} // namespace

OnboardingController::OnboardingController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // Compiled into the core: no device, no lock, fine on this thread
    m_gettingStarted = coreJson(dspi_onboarding_getting_started_json, 16384).object().toVariantMap();
    m_releases = coreJson(dspi_onboarding_whats_new_json, 16384).array().toVariantList();
    // Newest first whatever the file's order (the Mac's rule)
    std::stable_sort(m_releases.begin(), m_releases.end(), [](const QVariant &a, const QVariant &b) {
        return Version::parse(b.toMap().value(QStringLiteral("version")).toString())
               < Version::parse(a.toMap().value(QStringLiteral("version")).toString());
    });
    m_licence = coreJson(dspi_about_json, 8192).object().value(QLatin1String("licence")).toObject().toVariantMap();

    m_wizard = !setupDone();
    ctx.shell->addHandler(QStringLiteral("help.gettingStarted"), this, [this]() { requestSetup(); });
    if (diagnosticsAvailable())
        ctx.shell->addHandler(QStringLiteral("help.saveDiagnostics"), this, [this]() { saveDiagnostics(); });
    // After every controller and the QML exist: the window opens through the shell
    QMetaObject::invokeMethod(this, &OnboardingController::showReleaseNotesIfUnread, Qt::QueuedConnection);
}

OnboardingController *OnboardingController::create(QQmlEngine *, QJSEngine *) {
    OnboardingController *self = Registrar::instance<OnboardingController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

bool OnboardingController::setupDone() const { return QSettings().value(QLatin1String(kSetupDoneKey), false).toBool(); }

QString OnboardingController::whatsNewShown() const { return QSettings().value(QLatin1String(kWhatsNewKey)).toString(); }

QString OnboardingController::bundledFirmware() const { return Version::parse(QStringLiteral(DSPI_BUNDLED_FIRMWARE)).text(); }

void OnboardingController::showReleaseNotesIfUnread() {
    // Release notes are not onboarding: a new install has nothing to catch up
    // on and starts caught up; after an update the unread notes open once
    const QString shown = whatsNewShown();
    if (shown.isEmpty()) markWhatsNewRead();
    else if (!unreadSince(shown).isEmpty()) m_ctx.shell->openWindow(QStringLiteral("help.whatsNew"));
}

void OnboardingController::setInstaller(QObject *installer) {
    if (installer == m_installer) return;
    m_installer = installer;
    emit installerChanged();
}

void OnboardingController::requestSetup() {
    if (m_wizard) return;
    m_wizard = true;
    emit wizardShownChanged();
}

void OnboardingController::finishSetup() {
    QSettings().setValue(QLatin1String(kSetupDoneKey), true);
    if (!m_wizard) return;
    m_wizard = false;
    emit wizardShownChanged();
}

QString OnboardingController::firmwareMatch(const QString &deviceVersion) const {
    const Version device = Version::parse(deviceVersion);
    const Version bundled = Version::parse(QStringLiteral(DSPI_BUNDLED_FIRMWARE));
    if (!device.valid || !bundled.valid) return {};
    if (device == bundled) return QStringLiteral("match");
    return device < bundled ? QStringLiteral("older") : QStringLiteral("newer");
}

QVariantList OnboardingController::unreadSince(const QString &shown) const {
    QVariantList out;
    const Version from = Version::parse(shown);
    const Version cap = Version::parse(QStringLiteral(DSPI_BUNDLED_FIRMWARE));
    if (!from.valid || !cap.valid) return out;
    for (const QVariant &r : m_releases) {
        const Version v = Version::parse(r.toMap().value(QStringLiteral("version")).toString());
        if (v.valid && from < v && !(cap < v)) out << r;
    }
    return out;
}

void OnboardingController::markWhatsNewRead() {
    const QString bundled = bundledFirmware();
    if (bundled.isEmpty() || bundled == whatsNewShown()) return;
    QSettings().setValue(QLatin1String(kWhatsNewKey), bundled);
    emit whatsNewShownChanged();
}

void OnboardingController::resetOnboarding() {
    QSettings s;
    s.remove(QLatin1String(kSetupDoneKey));
    s.remove(QLatin1String(kWhatsNewKey));
    emit whatsNewShownChanged();
    emit wizardShownChanged();
}

// ---- Help > Save Diagnostics... ----------------------------------------------
// The core's bundle (dspi_diagnostics_save, PRO1 E2) is on main with P0E; a
// core without it builds this app with the row hidden (cmake/wp/qp17.cmake
// looks for the function in dspi_core.h and sets DSPI_HAVE_DIAGNOSTICS).

bool OnboardingController::diagnosticsAvailable() const {
#ifdef DSPI_HAVE_DIAGNOSTICS
    return true;
#else
    return false;
#endif
}

void OnboardingController::saveDiagnostics() {
#ifdef DSPI_HAVE_DIAGNOSTICS
    if (m_saving) return;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            return QString::fromUtf8(readSized([c](char *buf, uint32_t len) { return dspi_diagnostics_file_name(c, buf, len); }, 128));
        },
        [this](const QString &name) {
            const QString start = QDir(QStandardPaths::writableLocation(QStandardPaths::DocumentsLocation)).filePath(name);
            FilesController::pickFile(true, tr("Save Diagnostics"), tr("Zip archives (*.zip)"), start, this,
                                      [this](const QString &path) { saveDiagnosticsTo(path); });
        });
#endif
}

void OnboardingController::saveDiagnosticsTo(const QString &path) {
#ifdef DSPI_HAVE_DIAGNOSTICS
    if (m_saving || path.isEmpty()) return;
    m_saving = true;
    emit savingDiagnosticsChanged();
    const QJsonObject options{
        { QStringLiteral("app"), AppIdentity::displayName() + QLatin1Char(' ') + AppIdentity::version() },
        { QStringLiteral("hubStoreDir"),
          QDir(QStandardPaths::writableLocation(QStandardPaths::AppDataLocation)).filePath(QStringLiteral("hub")) },
    };
    const QByteArray file = QDir::toNativeSeparators(path).toUtf8();
    const QByteArray opts = QJsonDocument(options).toJson(QJsonDocument::Compact);
    m_ctx.core->worker()->call(
        this,
        [file, opts](FfiCore *c) {
            // The bundle is written on every call, so one buffer is enough
            QByteArray reply(1024, '\0');
            dspi_diagnostics_save(c, file.constData(), opts.constData(), reply.data(), uint32_t(reply.size()));
            return QJsonDocument::fromJson(QByteArray(reply.constData())).object();
        },
        [this, path](const QJsonObject &reply) {
            m_saving = false;
            emit savingDiagnosticsChanged();
            if (reply.value(QLatin1String("ok")).toBool())
                emit notice(tr("Diagnostics Saved"),
                            tr("The diagnostics were saved to %1. Nothing was sent anywhere; attach the file to a report "
                               "if you want to share it.")
                                .arg(QDir::toNativeSeparators(path)),
                            false);
            else
                emit notice(tr("Diagnostics Not Saved"),
                            reply.value(QLatin1String("error")).toString().isEmpty()
                                ? tr("The bundle could not be written.")
                                : reply.value(QLatin1String("error")).toString(),
                            true);
        });
#else
    Q_UNUSED(path)
#endif
}
