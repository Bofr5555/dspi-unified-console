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
class FilesController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // A configuration is being written to the device (the progress sheet)
    Q_PROPERTY(bool importing READ importing NOTIFY importingChanged)
public:
    FilesController(const AppContext &ctx, QObject *parent = nullptr);
    static FilesController *create(QQmlEngine *, QJSEngine *);

    bool importing() const { return m_importing; }

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
    Q_INVOKABLE void applyConfiguration(bool volumes, bool hardware);
    Q_INVOKABLE void cancelImport();

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
    // Import Device Configuration's options, under `prompt`
    void configurationOptions(const QString &prompt);
    // An alert: `error` gives it the error look
    void notice(const QString &title, const QString &text, bool error);

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
};

#endif // FILESCONTROLLER_H
