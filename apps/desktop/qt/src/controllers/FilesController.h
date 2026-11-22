#ifndef FILESCONTROLLER_H
#define FILESCONTROLLER_H

#include <QObject>
#include <QPointer>
#include <QQuickItem>
#include <QStringList>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class QFileDialog;
class QJsonObject;
class DeviceSession;
class QJSEngine;
class QQmlEngine;
class QQuickWindow;

// The File menu (DSPi_ConsoleApp.swift's FileMenuActions): filter files and
// device configurations (.dspipreset) in and out, Save Master Volume and Save
// Output Configuration. The core reads and writes both formats
// (dspi_filters_*, dspi_config_*); this picks the files, asks what to bring in
// and shows the core's report.
//
// - Import Filters: a Console file offers each channel it holds, all ticked; a
//   REW or AutoEQ list offers the active inputs (ticked) and the outputs.
// - Import Device Configuration: audio processing always, volumes and hardware
//   I/O when ticked; the input-pair links the document carries are restored.
//
// The questions and alerts are files/FilesHost.qml's, made in the main window.
// Files are read and written on the core's worker. PRO1's snapshots and
// safe-apply hook in at applyConfiguration(): the one place a document
// reaches the device.
//
// -- WP P1QT1 -- (PLAN-CLIENTS 5.9: P1DF, P1SA, P1OF)
// - The import asks with the core's preview: what each block would change on
//   the device (dspi_config_preview), a box per block, and "Apply with
//   rollback" (dspi_apply_guarded_import; ticked and fixed over DSPi Link to a
//   hub with the guard). The countdown bar is SnapshotsController's.
// - Open File… opens a document with no device (dspi_open_document): every
//   screen edits it, the device button says "File: <name>", Save File writes
//   it back. New from Template… opens a template the same way; Save as
//   Template… writes the open device or document as one, in the app's
//   templates folder by default.
class FilesController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // A configuration is being written to the device (the progress sheet)
    Q_PROPERTY(bool importing READ importing NOTIFY importingChanged)
    // -- WP P1QT1 -- an offline document is open, and its file's name
    Q_PROPERTY(bool offline READ offline NOTIFY offlineChanged)
    Q_PROPERTY(QString offlineFile READ offlineFile NOTIFY offlineChanged)
public:
    FilesController(const AppContext &ctx, QObject *parent = nullptr);
    static FilesController *create(QQmlEngine *, QJSEngine *);

    bool importing() const { return m_importing; }
    bool offline() const { return m_offline; }               // -- WP P1QT1 --
    QString offlineFile() const { return m_offlineFile; }

    // The menu rows' work after the file dialog, by path (the smoke driver
    // calls these directly too)
    Q_INVOKABLE void importFiltersFrom(const QString &path);
    Q_INVOKABLE void exportFiltersTo(const QString &path);
    Q_INVOKABLE void importConfigurationFrom(const QString &path);
    Q_INVOKABLE void exportConfigurationTo(const QString &path);
    Q_INVOKABLE void saveMasterVolume();
    Q_INVOKABLE void saveOutputConfiguration();

    // The pickers' answers. `wires`: the ticked channels; an empty list imports nothing.
    Q_INVOKABLE void applyFilters(const QVariantList &wires);
    // -- WP P1QT1 -- `blocks`: the DSPI_IMPORT_* block bits ticked; `guarded`:
    // with rollback (Keep or Revert)
    Q_INVOKABLE void applyConfiguration(int blocks, bool guarded);
    Q_INVOKABLE void cancelImport();
    // -- WP QPI2 -- the Device menu's question answered: `op` as deviceQuestion gave it
    Q_INVOKABLE void answerDevice(const QString &op, const QString &answer);
    // -- WP P1QT1 -- offline documents and templates, by path or id (the menu
    // rows' work after their dialogs; the smoke driver calls these too)
    Q_INVOKABLE void openFileFrom(const QString &path);
    Q_INVOKABLE void saveFile();
    Q_INVOKABLE void saveFileTo(const QString &path);
    Q_INVOKABLE void saveTemplateTo(const QString &path);
    Q_INVOKABLE void listTemplates();
    // Template `id` (dspi_template_list's), shown as `name`
    Q_INVOKABLE void openTemplate(const QString &id, const QString &name);
    // Where Save as Template… writes and New from Template… also looks
    static QString templatesDir();

    // A file dialog (the system's; the portal's on Linux) that calls `done`
    // with the chosen path. `save`: a save dialog starting at `name`.
    static void pickFile(bool save, const QString &title, const QString &filters, const QString &name,
                         QObject *ctx, std::function<void(const QString &)> done);
    // The open file dialog, for the smoke driver
    static QFileDialog *openDialog();
    // The main window's host of the questions and alerts, for the smoke driver
    QQuickItem *host() const { return m_host; }

signals:
    void importingChanged();
    // Import Filters' picker: `channels` [{wire, label, checked}]
    void filtersPicker(const QString &prompt, const QVariantList &channels);
    // Import Device Configuration's options, under `prompt`. -- WP P1QT1 --
    // `preview`: the core's {rows, blocks, summary, count} for every block;
    // `guardDefault`: the import is guarded whatever is ticked
    void configurationOptions(const QString &prompt, const QVariantMap &preview, bool guardDefault);
    // -- WP P1QT1 -- New from Template…'s list: [{id, name, builtin, platform, inputs, outputs}]
    void templatesPicker(const QVariantList &templates);
    void offlineChanged();
    // An alert: `error` gives it the error look
    void notice(const QString &title, const QString &text, bool error);
    // -- WP QPI2 -- Commit, Revert to Saved, Factory Reset ask first; `op`
    // goes back in answerDevice()
    void deviceQuestion(const QString &op, const QString &title, const QString &text, const QString &acceptText,
                        bool destructive);

private:
    void wire(QQuickWindow *window);
    void showError(const QString &text) { emit notice(tr("Error"), text, true); }
    void showSuccess(const QString &text) { emit notice(tr("Success"), text, false); }
    bool needDevice();
    QString refusal(const QJsonObject &reply) const;
    void setImporting(bool importing);
    QString importPrompt(const QString &platform, const QString &firmware, const QString &savedUtc) const;

    AppContext m_ctx;
    QPointer<QQuickItem> m_host;
    QByteArray m_pending;   // the file text a picker is open for
    bool m_importing = false;
    // -- WP P1QT1 --
    void openDocument(const QByteArray &text, const QString &fileName, const QString &path);
    void readOffline();
    QString saveName() const;
    bool m_offline = false;
    QString m_offlineFile;
    QString m_documentPath;   // where Save File writes; "" for a template
    // -- WP MDQT2b -- the device a question (an import's options, a Device menu
    // question) was asked about: its answer goes there, or nowhere
    QPointer<DeviceSession> m_askedOf;
    bool stillAskedOf();
};

#endif // FILESCONTROLLER_H
