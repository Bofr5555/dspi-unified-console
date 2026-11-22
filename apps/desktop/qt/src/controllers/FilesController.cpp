// -- WP QP15 -- The File menu: filters and device configurations in and out,
// Save Master Volume, Save Output Configuration
#include "FilesController.h"
#include "Confirm.h"   // -- WP P1QT2 --

#include "AppIdentity.h"
#include "ChannelController.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceSession.h"   // -- WP MDQT2b --
#include "HardwareController.h"
#include "PresetModel.h"
#include "Shell.h"
#include "Snapshot.h"
#include "LinkController.h"        // -- WP P1QT1 --
#include "NoticeCenter.h"
#include "SnapshotsController.h"
#include "UnsavedGuard.h"

#include "CoreApi.h"

#include <QDateTime>
#include <QPointer>
#include <QDir>   // -- WP P1QT1 --
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickWindow>
#include <QSaveFile>
#include <QStandardPaths>   // -- WP P1QT1 --
#include <memory>

static const Registrar::Controller<FilesController> filesController;
static const Registrar::Banner fileBanner("FileBanner", 36);   // -- WP P1QT1 --

namespace {
// The core refuses a file over 4 MB; one byte more is enough for it to say so
constexpr qint64 kReadLimit = (4 << 20) + 1;
// An import's report: far more than its few lines need
constexpr uint32_t kReplyBytes = 256 << 10;
constexpr uint8_t kStatusLocked = 0xFE;
constexpr double kMuteDb = -128.0;

QPointer<QFileDialog> s_dialog;

struct Read {
    QByteArray text;
    QString error;
};

// A file's bytes, or why not. Worker thread.
Read readFile(const QString &path) {
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) return { {}, FilesController::tr("Failed to read file: %1").arg(f.errorString()) };
    return { f.read(kReadLimit), {} };
}

// Writes `bytes` in one go (a temporary file renamed over the target); the error, or "".
QString writeFile(const QString &path, const QByteArray &bytes) {
    QSaveFile f(path);
    if (!f.open(QIODevice::WriteOnly) || f.write(bytes) != bytes.size() || !f.commit())
        return FilesController::tr("Failed to write file: %1").arg(f.errorString());
    return {};
}

// One call of a function that changes the device on every call: never twice
// for a short buffer, as readSized would. Worker thread.
QByteArray callOnce(const std::function<uint32_t(char *, uint32_t)> &fill) {
    QByteArray buf(int(kReplyBytes), '\0');
    const uint32_t need = fill(buf.data(), kReplyBytes);
    if (need == 0 || need > kReplyBytes) return QByteArray();
    buf.truncate(int(need) - 1);
    return buf;
}

// An export replies the document, or a JSON failure: its sentence, or ""
QString exportFailure(const QByteArray &reply) {
    if (reply.isEmpty()) return FilesController::tr("No device connected.");
    if (!reply.startsWith('{')) return {};
    const QJsonObject o = QJsonDocument::fromJson(reply).object();
    if (!o.contains(QLatin1String("ok")) || o.value(QLatin1String("ok")).toBool()) return {};
    return o.value(QLatin1String("error")).toString();
}

// The core's report lines as the Mac's alerts set them: the first, then the
// notes under it
QString reportText(const QJsonArray &lines) {
    QStringList all;
    for (const QJsonValue &v : lines) all.append(v.toString());
    if (all.isEmpty()) return {};
    const QString first = all.takeFirst();
    return all.isEmpty() ? first : first + QStringLiteral("\n\n") + all.join(QLatin1Char('\n'));
}

int activeBands(const QJsonValue &bank) {
    int n = 0;
    for (const QJsonValue &b : bank.toArray())
        if (b.toObject().value(QLatin1String("type")).toInt() != 0) ++n;
    return n;
}
} // namespace

FilesController::FilesController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    Shell *shell = ctx.shell;
    shell->addHandler(QStringLiteral("file.importFilters"), this, [this] {
        pickFile(false, tr("Import Filters"), tr("Text files (*.txt);;All files (*)"), QString(), this,
                 [this](const QString &path) { importFiltersFrom(path); });
    });
    shell->addHandler(QStringLiteral("file.exportFilters"), this, [this] {
        pickFile(true, tr("Export Filters"), tr("Text files (*.txt)"), tr("DSPi Filters.txt"), this,
                 [this](const QString &path) { exportFiltersTo(path); });
    });
    shell->addHandler(QStringLiteral("file.importConfiguration"), this, [this] {
        if (!needDevice()) return;
        pickFile(false, tr("Import Device Configuration"), tr("DSPi configurations (*.dspipreset *.json);;All files (*)"),
                 QString(), this, [this](const QString &path) { importConfigurationFrom(path); });
    });
    shell->addHandler(QStringLiteral("file.exportConfiguration"), this, [this] {
        if (!needDevice()) return;
        // The document's channel ids depend on the platform, so a file written
        // before the device has identified itself could land on the wrong channels
        if (m_ctx.device->platformName().isEmpty()) {
            showError(tr("Still identifying the device. Try again in a moment."));
            return;
        }
        pickFile(true, tr("Export Device Configuration"), tr("DSPi configurations (*.dspipreset)"),
                 tr("DSPi Configuration") + QStringLiteral(".dspipreset"), this,
                 [this](const QString &path) { exportConfigurationTo(path); });
    });
    shell->addHandler(QStringLiteral("file.saveMasterVolume"), this, [this] { saveMasterVolume(); });
    shell->addHandler(QStringLiteral("file.saveOutputConfiguration"), this, [this] { saveOutputConfiguration(); });
    // -- WP QPI2 -- the Device menu's flash rows (the Mac's ToolsMenuActions):
    // each asks first, then works on the active preset
    shell->addHandler(QStringLiteral("device.commit"), this, [this] {
        if (!needDevice()) return;
        const int slot = m_ctx.device->presets()->activeSlot();
        if (slot < 0) {
            showError(tr("Still identifying the device. Try again in a moment."));
            return;
        }
        m_askedOf = m_ctx.activeSession();   // -- WP MDQT2b --
        emit deviceQuestion(QStringLiteral("commit"), tr("Save Preset"),
                            tr("Save current parameters to preset slot %1?").arg(slot + 1), tr("Save"), false);
    });
    shell->addHandler(QStringLiteral("device.revert"), this, [this] {
        m_askedOf = m_ctx.activeSession();   // -- WP MDQT2b --
        emit deviceQuestion(QStringLiteral("revert"), tr("Revert to Saved"),
                            tr("Revert to last saved parameters?\n\nCurrent unsaved changes will be lost."), tr("Revert"),
                            false);
    });
    shell->addHandler(QStringLiteral("device.factoryReset"), this, [this] {
        m_askedOf = m_ctx.activeSession();   // -- WP MDQT2b --
        emit deviceQuestion(QStringLiteral("factoryReset"), tr("Factory Reset"),
                            tr("Do you wish to clear all active parameters?\n\nThis will not overwrite your saved "
                               "parameters unless you run 'Commit Parameters'."),
                            tr("Reset"), true);
    });

    // -- WP P1QT1 -- offline documents and templates (P1OF)
    shell->addHandler(QStringLiteral("file.openFile"), this, [this] {
        pickFile(false, tr("Open File"), tr("DSPi configurations (*.dspipreset *.json);;All files (*)"), QString(), this,
                 [this](const QString &path) { openFileFrom(path); });
    });
    shell->addHandler(QStringLiteral("file.newFromTemplate"), this, [this] { listTemplates(); });
    shell->addHandler(QStringLiteral("file.saveFile"), this, [this] { saveFile(); });
    shell->addHandler(QStringLiteral("file.saveAsTemplate"), this, [this] {
        if (!needDevice()) return;
        if (m_ctx.device->platformName().isEmpty()) {
            showError(tr("Still identifying the device. Try again in a moment."));
            return;
        }
        QDir().mkpath(templatesDir());
        pickFile(true, tr("Save as Template"), tr("DSPi configurations (*.dspipreset)"),
                 templatesDir() + QLatin1Char('/') + tr("Template") + QStringLiteral(".dspipreset"), this,
                 [this](const QString &path) { saveTemplateTo(path); });
    });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &FilesController::readOffline);

    // The questions and alerts live in the main window, made once it exists
    if (auto *app = qobject_cast<QQmlApplicationEngine *>(ctx.engine)) {
        connect(app, &QQmlApplicationEngine::objectCreated, this, [this](QObject *object, const QUrl &) {
            if (auto *window = qobject_cast<QQuickWindow *>(object)) wire(window);
        });
    }
}

FilesController *FilesController::create(QQmlEngine *, QJSEngine *) {
    FilesController *self = Registrar::instance<FilesController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void FilesController::wire(QQuickWindow *window) {
    if (m_host) return;
    QQmlComponent component(m_ctx.engine, QStringLiteral("UConsole"), QStringLiteral("FilesHost"));
    QObject *made = component.createWithInitialProperties(
        { { QStringLiteral("parent"), QVariant::fromValue(window->contentItem()) } });
    m_host = qobject_cast<QQuickItem *>(made);
    if (!m_host) {
        qWarning("FilesController: the file dialogs' host did not load: %s", qPrintable(component.errorString()));
        delete made;
        return;
    }
    m_host->setParent(window->contentItem());
}

void FilesController::pickFile(bool save, const QString &title, const QString &filters, const QString &name,
                               QObject *ctx, std::function<void(const QString &)> done) {
    if (s_dialog && s_dialog->isVisible()) {
        s_dialog->raise();
        return;
    }
    auto *dialog = new QFileDialog(nullptr, title, QString(), filters);
    dialog->setAttribute(Qt::WA_DeleteOnClose);
    dialog->setWindowModality(Qt::ApplicationModal);
    dialog->setAcceptMode(save ? QFileDialog::AcceptSave : QFileDialog::AcceptOpen);
    dialog->setFileMode(save ? QFileDialog::AnyFile : QFileDialog::ExistingFile);
    if (!name.isEmpty()) dialog->selectFile(name);
    QPointer<QObject> guard(ctx);
    QObject::connect(dialog, &QFileDialog::fileSelected, dialog, [guard, done](const QString &path) {
        if (guard && !path.isEmpty()) done(path);
    });
    // Gone from here once answered; Qt deletes it later
    QObject::connect(dialog, &QDialog::finished, dialog, [dialog] {
        if (s_dialog == dialog) s_dialog = nullptr;
    });
    s_dialog = dialog;
    dialog->open();
}

QFileDialog *FilesController::openDialog() { return s_dialog; }

bool FilesController::needDevice() {
    if (m_ctx.device->connected()) return true;
    showError(tr("No device connected."));
    return false;
}

void FilesController::setImporting(bool importing) {
    if (importing == m_importing) return;
    m_importing = importing;
    emit importingChanged();
}

// ---- Filters --------------------------------------------------------------------

void FilesController::importFiltersFrom(const QString &path) {
    const auto inputs = uint8_t(m_ctx.device->numInputs());
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this,
        [path, inputs](FfiCore *c) {
            Read r = readFile(path);
            QJsonObject inspected;
            if (r.error.isEmpty()) {
                const QByteArray text = r.text;
                inspected = QJsonDocument::fromJson(readSized([c, text, inputs](char *b, uint32_t n) {
                                return dspi_filters_inspect(c, text.constData(), inputs, b, n);
                            })).object();
            }
            return std::make_pair(r, inspected);
        },
        [this, asked](std::pair<Read, QJsonObject> result) {
            if (asked != m_ctx.activeSession()) return;   // -- WP MDQT2b -- inspected for the device left
            const Read &r = result.first;
            const QJsonObject &o = result.second;
            if (!r.error.isEmpty()) {
                showError(r.error);
                return;
            }
            const QString kind = o.value(QLatin1String("kind")).toString();
            const QJsonArray channels = o.value(QLatin1String("channels")).toArray();
            if (kind == QLatin1String("error")) {
                showError(o.value(QLatin1String("error")).toString());
                return;
            }
            QString prompt;
            if (kind == QLatin1String("native")) {
                if (channels.isEmpty()) {
                    showError(tr("Failed to parse DSPi Console filter file"));
                    return;
                }
                prompt = tr("This file contains filter settings for multiple channels. Select which channels to import:");
            } else {
                const int filters = o.value(QLatin1String("filters")).toInt();
                if (filters == 0 || channels.isEmpty()) {
                    showError(tr("No valid filters found in file"));
                    return;
                }
                const QJsonValue preamp = o.value(QLatin1String("preamp"));
                prompt = preamp.isDouble()
                             //: %1 the number of filters, %2 the preamp in dB with its sign ("-6.5")
                             ? tr("Found %1 filter(s) and a %2 dB preamp. Select which channel(s) to apply them to:")
                                   .arg(filters)
                                   .arg(QString::asprintf("%+.1f", preamp.toDouble()))
                             : tr("Found %1 filter(s). Select which channel(s) to apply them to:").arg(filters);
            }
            QVariantList choices;
            for (const QJsonValue &v : channels) {
                const QJsonObject ch = v.toObject();
                choices.append(QVariantMap{ { QStringLiteral("wire"), ch.value(QLatin1String("wire")).toInt() },
                                            { QStringLiteral("label"), ch.value(QLatin1String("label")).toString() },
                                            { QStringLiteral("checked"), ch.value(QLatin1String("checked")).toBool() } });
            }
            m_pending = r.text;
            m_askedOf = asked;   // -- WP MDQT2b --
            emit filtersPicker(prompt, choices);
        });
}

void FilesController::applyFilters(const QVariantList &wires) {
    const QByteArray text = std::exchange(m_pending, QByteArray());
    if (text.isEmpty() || wires.isEmpty() || !stillAskedOf()) return;
    QByteArray chosen;
    for (const QVariant &w : wires) chosen.append(char(uint8_t(w.toInt())));
    auto reply = std::make_shared<QJsonObject>();
    m_ctx.device->write(
        QStringLiteral("filters import"),
        [text, chosen, reply](FfiCore *c) {
            *reply = QJsonDocument::fromJson(callOnce([c, &text, &chosen](char *b, uint32_t n) {
                         return dspi_filters_import(c, text.constData(),
                                                    reinterpret_cast<const uint8_t *>(chosen.constData()),
                                                    uint32_t(chosen.size()), b, n);
                     })).object();
            return reply->value(QLatin1String("ok")).toBool();
        },
        [this, reply](bool ok) {
            if (ok) showSuccess(reportText(reply->value(QLatin1String("lines")).toArray()));
            else showError(refusal(*reply));
        });
}

// Why a write did not happen: the core's sentence, or the write path's refusal
QString FilesController::refusal(const QJsonObject &reply) const {
    const QString error = reply.value(QLatin1String("error")).toString();
    if (!error.isEmpty()) return error;
    if (m_ctx.device->readOnly()) return tr("This device's firmware is read-only in this version; nothing was written.");
    return tr("No device connected.");
}

void FilesController::exportFiltersTo(const QString &path) {
    const auto inputs = uint8_t(m_ctx.device->numInputs());
    m_ctx.core->worker()->call(
        this,
        [path, inputs](FfiCore *c) {
            const QByteArray doc = readSized([c, inputs](char *b, uint32_t n) { return dspi_filters_export(c, inputs, b, n); });
            const QString failure = exportFailure(doc);
            return failure.isEmpty() ? writeFile(path, doc) : failure;
        },
        [this](QString error) {
            if (error.isEmpty()) showSuccess(tr("Filters exported successfully"));
            else showError(error);
        });
}

// ---- Device configurations ---------------------------------------------------------

void FilesController::exportConfigurationTo(const QString &path) {
    // The document's name is the file's, as the Mac takes it from the save panel
    const QByteArray name = QFileInfo(path).completeBaseName().toUtf8();
    const QByteArray version = AppIdentity::version().toUtf8();
    const ChannelController *channels = Registrar::instance<ChannelController>();
    const auto links = uint8_t(channels ? channels->linkedPairs() : 0);
    m_ctx.core->worker()->call(
        this,
        [path, name, version, links](FfiCore *c) {
            const QByteArray doc = readSized([&](char *b, uint32_t n) {
                return dspi_config_export(c, name.constData(), version.constData(), links, b, n);
            });
            const QString failure = exportFailure(doc);
            return std::make_pair(failure.isEmpty() ? writeFile(path, doc) : failure, doc);
        },
        [this](std::pair<QString, QByteArray> r) {
            if (!r.first.isEmpty()) {
                showError(r.first);
                return;
            }
            // Bands that do something, not the empty slots padding every bank
            const QJsonObject doc = QJsonDocument::fromJson(r.second).object();
            const QJsonArray channels = doc.value(QLatin1String("channels")).toArray();
            int bands = 0, xover = 0;
            for (const QJsonValue &ch : channels) {
                bands += activeBands(ch.toObject().value(QLatin1String("eq")));
                xover += activeBands(ch.toObject().value(QLatin1String("crossover")));
            }
            const int crosspoints = int(doc.value(QLatin1String("matrix")).toArray().size());
            showSuccess(tr("Configuration exported.\n\n%1 channels, %2 active EQ bands, %3 crossover bands, %4 crosspoints.")
                            .arg(channels.size())
                            .arg(bands)
                            .arg(xover)
                            .arg(crosspoints));
        });
}

// What the options ask under their heading: where the file came from, and a
// warning when it is from another platform (configurationImportPrompt)
QString FilesController::importPrompt(const QString &platform, const QString &firmware, const QString &savedUtc) const {
    QStringList from;
    if (!platform.isEmpty()) from.append(platform);
    if (!firmware.isEmpty()) from.append(tr("firmware %1").arg(firmware));
    const QDateTime saved = QDateTime::fromString(savedUtc, Qt::ISODate);
    if (saved.isValid()) from.append(saved.toLocalTime().toString(QStringLiteral("yyyy-MM-dd HH:mm")));
    QString text = from.isEmpty() ? tr("Applies the settings in this file to the connected device.")
                                  : tr("Saved from %1.").arg(from.join(QStringLiteral(", ")));
    const QString here = m_ctx.device->platformName();
    if (!platform.isEmpty() && !here.isEmpty() && platform.compare(here, Qt::CaseInsensitive) != 0)
        text += QStringLiteral("\n\n") +
                tr("This file came from a %1 device and you are connected to %2. Anything the connected device doesn't have will be skipped.")
                    .arg(platform, here);
    // -- WP P1QT1 -- the blocks are chosen below the prompt now
    return text;
}

void FilesController::importConfigurationFrom(const QString &path) {
    if (!needDevice()) return;
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this,
        [path](FfiCore *c) {
            Read r = readFile(path);
            QJsonObject inspected;
            // -- WP P1QT1 -- what every block would change, and whether the
            // import is guarded here by default
            QJsonObject preview;
            bool guardDefault = false;
            if (r.error.isEmpty()) {
                const QByteArray text = r.text;
                inspected = QJsonDocument::fromJson(readSized([text](char *b, uint32_t n) {
                                return dspi_config_inspect(text.constData(), b, n);
                            })).object();
                if (inspected.value(QLatin1String("ok")).toBool()) {
                    preview = QJsonDocument::fromJson(readSized([c, text](char *b, uint32_t n) {
                                  return dspi_config_preview(c, text.constData(), IMPORT_BY_BLOCK | IMPORT_ALL_BLOCKS,
                                                             0xFFFFFFFFu, b, n);
                              })).object();
                    guardDefault = dspi_apply_guard_default(c);
                }
            }
            inspected.insert(QStringLiteral("_preview"), preview);
            inspected.insert(QStringLiteral("_guardDefault"), guardDefault);
            return std::make_pair(r, inspected);
        },
        [this, asked](std::pair<Read, QJsonObject> result) {
            if (asked != m_ctx.activeSession()) return;   // -- WP MDQT2b -- inspected for the device left
            const QJsonObject &o = result.second;
            if (!result.first.error.isEmpty()) {
                showError(result.first.error);
                return;
            }
            if (!o.value(QLatin1String("ok")).toBool()) {
                showError(o.value(QLatin1String("error")).toString());
                return;
            }
            // -- WP P1QT1 -- a preview the core refused (no settings read yet) is said, not shown empty
            const QJsonObject preview = o.value(QLatin1String("_preview")).toObject();
            if (preview.contains(QLatin1String("ok")) && !preview.value(QLatin1String("ok")).toBool()) {
                showError(preview.value(QLatin1String("error")).toString());
                return;
            }
            m_pending = result.first.text;
            m_askedOf = asked;   // -- WP MDQT2b --
            emit configurationOptions(importPrompt(o.value(QLatin1String("platform")).toString(),
                                                   o.value(QLatin1String("firmware")).toString(),
                                                   o.value(QLatin1String("savedUtc")).toString()),
                                      preview.toVariantMap(), o.value(QLatin1String("_guardDefault")).toBool());
        });
}

// The one place a configuration document reaches the device (PRO1's snapshot
// before and safe-apply hook in here)
void FilesController::applyConfiguration(int blocks, bool guarded) {
    const QByteArray text = std::exchange(m_pending, QByteArray());
    if (text.isEmpty() || !stillAskedOf()) return;
    // -- WP P1QT1 -- by block (P1DF); every channel's EQ when EQ is ticked
    const uint32_t options = IMPORT_BY_BLOCK | (uint32_t(blocks) & IMPORT_ALL_BLOCKS);
    if ((options & IMPORT_ALL_BLOCKS) == 0) {
        showError(tr("Nothing was selected to import; nothing was changed."));
        return;
    }
    const bool hardware = options & (IMPORT_HARDWARE | IMPORT_LIMITERS);
    auto reply = std::make_shared<QJsonObject>();
    // -- WP QPI2 -- the hardware I/O (pins, limiters) is output configuration:
    // in Independent mode it lands live and unsaved, so the save bar offers it
    if (hardware)
        if (auto *hw = Registrar::instance<HardwareController>()) hw->beginOutputEdit();
    setImporting(true);
    if (guarded) {
        // -- WP P1QT1 -- with rollback: the core captures a snapshot first, and
        // the countdown bar (SnapshotsController) offers Keep and Revert
        // -- WP P1DCFIX -- for the time set in the Snapshots window (30 s unless set)
        auto *snapshots = Registrar::instance<SnapshotsController>();
        const uint32_t timeout = snapshots ? uint32_t(snapshots->guardTimeoutS()) : 0;
        const QPointer<DeviceSession> session = m_ctx.activeSession();   // -- WP MDQT2b --
        m_ctx.device->write(
            QStringLiteral("configuration import"),
            [text, options, reply, timeout](FfiCore *c) {
                *reply = QJsonDocument::fromJson(callOnce([c, &text, options, timeout](char *b, uint32_t n) {
                             return dspi_apply_guarded_import(c, text.constData(), options, 0xFFFFFFFFu, timeout, b, n);
                         })).object();
                return reply->value(QLatin1String("ok")).toBool();
            },
            [this, reply, session](bool ok) {
                setImporting(false);
                if (!ok) {
                    showError(refusal(*reply));
                    return;
                }
                // -- WP MDQT2b -- with the device it was begun on
                if (auto *snapshots = Registrar::instance<SnapshotsController>()) snapshots->guardStarted(*reply, session);
            });
        return;
    }
    m_ctx.device->write(
        QStringLiteral("configuration import"),
        [text, options, reply](FfiCore *c) {
            // -- WP P1QT2 -- with a token: the import was confirmed in its dialog
            *reply = QJsonDocument::fromJson(Confirm::run(c, Confirm::kConfigImport, [c, &text, options](const char *t, char *b, uint32_t n) {
                         return dspi_config_import_confirmed(c, text.constData(), options, 0xFFFFFFFFu, t, b, n);
                     }, kReplyBytes).json).object();
            return reply->value(QLatin1String("ok")).toBool();
        },
        [this, reply](bool ok) {
            setImporting(false);
            if (!ok) {
                showError(refusal(*reply));
                return;
            }
            // The links first, as the Mac applies them before the channels
            // (here the core has written every channel already, so no mirroring
            // runs in between)
            const QJsonValue links = reply->value(QLatin1String("links"));
            if (ChannelController *channels = Registrar::instance<ChannelController>(); channels && links.isDouble()) {
                const int mask = links.toInt();
                for (int pair = 0; pair < MAX_INPUTS / 2; ++pair) {
                    const bool linked = mask >> pair & 1;
                    if (channels->isLinked(2 * pair) == linked) continue;
                    if (linked) channels->linkPair(2 * pair, -1);
                    else channels->unlinkPair(2 * pair);
                }
            }
            const QString text = reportText(reply->value(QLatin1String("lines")).toArray());
            // Anything refused or skipped is not a success, so no "Success" heading
            if (reply->value(QLatin1String("clean")).toBool()) showSuccess(text);
            else emit notice(tr("Import Device Configuration"), text, false);
        });
}

void FilesController::cancelImport() { m_pending.clear(); }

// -- WP MDQT2b -- The device a question was asked about is still the active
// one; when it is not, nothing is done, and that is said
bool FilesController::stillAskedOf() {
    const bool same = m_askedOf == m_ctx.activeSession();
    m_askedOf = nullptr;
    if (!same) showError(tr("Another device is shown now. Nothing was changed."));
    return same;
}

// ---- Saves -----------------------------------------------------------------------

void FilesController::saveMasterVolume() {
    if (!needDevice()) return;
    const DspState *s = m_ctx.device->state();
    const double db = s ? double(s->master_volume_db) : 0.0;
    m_ctx.device->write(QStringLiteral("master volume save"),
                        [](FfiCore *c) { return dspi_save_master_volume(c) == FLASH_OK; },
                        [this, db](bool ok) {
                            if (!ok) {
                                showError(tr("Failed to save master volume — the device did not acknowledge the request."));
                                return;
                            }
                            const QString shown = db <= kMuteDb ? tr("−∞ dB (mute)") : tr("%1 dB").arg(db, 0, 'f', 1);
                            showSuccess(tr("Master volume saved (%1). It will be applied on next boot.").arg(shown));
                        });
}

// -- WP QPI2 --
void FilesController::answerDevice(const QString &op, const QString &answer) {
    if (answer != QLatin1String("accept") || !stillAskedOf() || !needDevice()) return;
    const int slot = m_ctx.device->presets()->activeSlot();
    if (slot < 0) {
        showError(tr("Still identifying the device. Try again in a moment."));
        return;
    }
    auto status = std::make_shared<uint8_t>(0xFF);
    if (op == QLatin1String("commit")) {
        // A slot without a name gets "Preset N" first, as the sidebar's save does
        const QByteArray fallback = tr("Preset %1").arg(slot + 1).toUtf8();
        m_ctx.device->write(
            QStringLiteral("preset save"),
            [slot, fallback, status](FfiCore *c) {
                char name[CHANNEL_NAME_LEN] = {};
                if (!dspi_get_preset_name(c, uint8_t(slot), name, sizeof name) || name[0] == 0)
                    dspi_set_preset_name(c, uint8_t(slot), fallback.constData());
                *status = Confirm::savePreset(c, uint8_t(slot));   // -- WP P1QT2 -- token
                return *status == PRESET_OK;
            },
            [this, status](bool ok) {
                if (ok) showSuccess(tr("Preset saved successfully"));
                else
                    showError(tr("Failed to save preset (%1)")
                                  .arg(*status == kStatusLocked ? tr("Another console is applying changes")
                                                                : tr("error %1").arg(*status)));
            });
    } else if (op == QLatin1String("revert")) {
        // The active slot loaded again (the Mac's loadParams), the deferred
        // load that is safe on every input source
        m_ctx.device->write(
            QStringLiteral("preset load"),
            [slot, status](FfiCore *c) {
                *status = dspi_load_preset(c, uint8_t(slot));
                return *status == PRESET_OK;
            },
            [this, status](bool ok) {
                if (ok) showSuccess(tr("Parameters reverted successfully"));
                else if (*status == PRESET_ERR_CRC) showError(tr("Saved data is corrupted"));
                else showError(tr("Failed to load parameters"));
            });
    } else if (op == QLatin1String("factoryReset")) {
        m_ctx.device->write(
            QStringLiteral("factory reset"),
            [status](FfiCore *c) {
                *status = Confirm::factoryReset(c);   // -- WP P1QT2 -- token
                return *status == FLASH_OK;
            },
            [this](bool ok) {
                if (ok) showSuccess(tr("Factory reset complete"));
                else showError(tr("Failed to reset parameters"));
            });
    }
}

void FilesController::saveOutputConfiguration() {
    if (!needDevice()) return;
    auto status = std::make_shared<uint8_t>(0xFF);
    m_ctx.device->write(
        QStringLiteral("output configuration save"),
        [status](FfiCore *c) {
            *status = Confirm::saveOutputConfig(c);   // -- WP P1QT2 -- token
            return *status == FLASH_OK;
        },
        [this, status](bool ok) {
            if (ok) showSuccess(tr("Output configuration saved. It will be applied on next boot."));
            else if (*status == kStatusLocked)
                showError(tr("Failed to save output configuration: %1.").arg(tr("Another console is applying changes")));
            else showError(tr("Failed to save output configuration. The device did not acknowledge the request."));
        });
}

// ---- -- WP P1QT1 -- Offline documents and templates (P1OF) ------------------------------

QString FilesController::templatesDir() {
    return QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/templates");
}

// The name a saved document carries: the file's
QString FilesController::saveName() const { return QFileInfo(m_documentPath).completeBaseName(); }

void FilesController::readOffline() {
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_offline_document(c, b, n); }); },
        [this, asked](QByteArray reply) {
            if (asked != m_ctx.activeSession()) return;
            const QJsonObject o = QJsonDocument::fromJson(reply).object();
            const bool offline = o.value(QLatin1String("offline")).toBool();
            const QString file = offline ? o.value(QLatin1String("fileName")).toString() : QString();
            if (offline == m_offline && file == m_offlineFile) return;
            m_offline = offline;
            m_offlineFile = file;
            if (!offline) m_documentPath.clear();
            emit offlineChanged();
        });
}

void FilesController::openFileFrom(const QString &path) {
    m_ctx.core->worker()->call(
        this, [path](FfiCore *) { return readFile(path); },
        [this, path](Read r) {
            if (!r.error.isEmpty()) {
                showError(r.error);
                return;
            }
            openDocument(r.text, QFileInfo(path).fileName(), path);
        });
}

void FilesController::openDocument(const QByteArray &text, const QString &fileName, const QString &path) {
    // Another "device": the unsaved-changes question first, then off any hub session
    UnsavedGuard::before(QStringLiteral("device"), this, [this, text, fileName, path] {
        if (LinkController *link = Registrar::instance<LinkController>();
            link && (link->inSession() || m_ctx.device->remote()))
            link->leaveForLocal();
        const QByteArray name = fileName.toUtf8();
        m_ctx.core->worker()->call(
            this,
            [text, name](FfiCore *c) {
                return QJsonDocument::fromJson(callOnce([&](char *b, uint32_t n) {
                           return dspi_open_document(c, text.constData(), name.constData(), b, n);
                       })).object();
            },
            [this, path](QJsonObject o) {
                if (!o.value(QLatin1String("ok")).toBool()) {
                    showError(o.value(QLatin1String("error")).toString(tr("This file is no DSPi configuration.")));
                    return;
                }
                m_documentPath = path;
                m_ctx.device->adoptDocument(QStringLiteral("OFFLINE-DOCUMENT"));
                QStringList notes;
                for (const QJsonValue &v : o.value(QLatin1String("notes")).toArray()) notes.append(v.toString());
                if (!notes.isEmpty()) emit notice(tr("Open File"), notes.join(QLatin1Char('\n')), false);
            });
    });
}

void FilesController::saveFile() {
    if (!m_offline) {
        showError(tr("No file is open."));
        return;
    }
    if (m_documentPath.isEmpty()) {
        // A template opened as a new file: where it goes is asked once
        pickFile(true, tr("Save File"), tr("DSPi configurations (*.dspipreset)"),
                 (m_offlineFile.isEmpty() ? tr("DSPi Configuration") : QFileInfo(m_offlineFile).completeBaseName())
                     + QStringLiteral(".dspipreset"),
                 this, [this](const QString &path) { saveFileTo(path); });
        return;
    }
    saveFileTo(m_documentPath);
}

void FilesController::saveFileTo(const QString &path) {
    const QByteArray version = AppIdentity::version().toUtf8();
    const ChannelController *channels = Registrar::instance<ChannelController>();
    const auto links = uint8_t(channels ? channels->linkedPairs() : 0);
    m_ctx.core->worker()->call(
        this,
        [path, version, links](FfiCore *c) {
            const QByteArray doc = readSized([&](char *b, uint32_t n) { return dspi_save_document(c, version.constData(), links, b, n); });
            const QString failure = exportFailure(doc);
            return failure.isEmpty() ? writeFile(path, doc) : failure;
        },
        [this, path](QString error) {
            if (!error.isEmpty()) {
                showError(error);
                return;
            }
            m_documentPath = path;
            //: %1 is the file's name
            if (auto *notices = Registrar::instance<NoticeCenter>()) notices->post(tr("Saved %1.").arg(QFileInfo(path).fileName()));
        });
}

void FilesController::saveTemplateTo(const QString &path) {
    const QByteArray name = QFileInfo(path).completeBaseName().toUtf8();
    const QByteArray version = AppIdentity::version().toUtf8();
    const ChannelController *channels = Registrar::instance<ChannelController>();
    const auto links = uint8_t(channels ? channels->linkedPairs() : 0);
    m_ctx.core->worker()->call(
        this,
        [path, name, version, links](FfiCore *c) {
            const QByteArray doc = readSized([&](char *b, uint32_t n) {
                return dspi_save_as_template(c, name.constData(), version.constData(), links, b, n);
            });
            const QString failure = exportFailure(doc);
            return failure.isEmpty() ? writeFile(path, doc) : failure;
        },
        [this, path](QString error) {
            if (!error.isEmpty()) {
                showError(error);
                return;
            }
            //: %1 is the template's file name
            if (auto *notices = Registrar::instance<NoticeCenter>()) notices->post(tr("Template saved as %1.").arg(QFileInfo(path).fileName()));
        });
}

void FilesController::listTemplates() {
    const QByteArray dir = QDir::toNativeSeparators(templatesDir()).toUtf8();
    m_ctx.core->worker()->call(
        this, [dir](FfiCore *) { return readSized([&](char *b, uint32_t n) { return dspi_template_list(dir.constData(), b, n); }); },
        [this](QByteArray reply) {
            QVariantList rows;
            for (const QJsonValue &v : QJsonDocument::fromJson(reply).array()) rows.append(v.toObject().toVariantMap());
            emit templatesPicker(rows);
        });
}

void FilesController::openTemplate(const QString &id, const QString &name) {
    const QByteArray i = id.toUtf8();
    const QByteArray dir = QDir::toNativeSeparators(templatesDir()).toUtf8();
    m_ctx.core->worker()->call(
        this, [i, dir](FfiCore *) { return readSized([&](char *b, uint32_t n) { return dspi_template_text(i.constData(), dir.constData(), b, n); }); },
        [this, name](QByteArray text) {
            const QString failure = exportFailure(text);
            if (!failure.isEmpty()) {
                showError(failure);
                return;
            }
            // A new file from the template: Save File asks where
            openDocument(text, name, QString());
        });
}
