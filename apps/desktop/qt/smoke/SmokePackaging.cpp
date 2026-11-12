// -- WP QP20 -- Section qp20-notices: what About shows of the notices the
// packages stage (packaging/licences/stage.sh, qt/cmake/wp/qp20.cmake): the
// app's licence label beside the GPL, Lucide's licence by name, and a
// package's folder laid out as the .rpm installs it.
#include "SmokeDriver.h"

#include "AboutController.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>

namespace {

QVariantMap rowEndingWith(const QVariantList &rows, const char *path, int *index = nullptr) {
    for (int i = 0; i < rows.size(); ++i) {
        const QVariantMap row = rows.at(i).toMap();
        if (row.value(QStringLiteral("path")).toString().endsWith(QLatin1String(path))) {
            if (index) *index = i;
            return row;
        }
    }
    return {};
}

int countEndingWith(const QVariantList &rows, const char *path) {
    int n = 0;
    for (const QVariant &row : rows)
        if (row.toMap().value(QStringLiteral("path")).toString().endsWith(QLatin1String(path))) ++n;
    return n;
}

bool writeFile(const QString &path, const QByteArray &text) {
    QDir().mkpath(QFileInfo(path).path());
    QFile f(path);
    return f.open(QIODevice::WriteOnly) && f.write(text) == text.size();
}

void noticesSection(SmokeDriver &d) {
    auto *about = Registrar::instance<AboutController>();
    d.check(QStringLiteral("qp20: the About controller exists"), about != nullptr);
    if (!about) return;
    about->rescanNotices();
    const QVariantList rows = about->notices();

    // The build's notices (the copy beside it that the packages stage)
    const QVariantMap gpl = rowEndingWith(rows, "LICENSES/GPL-3.0.txt");
    d.check(QStringLiteral("qp20: the GPL's row carries the app's licence label, GPL-3.0-only"),
            gpl.value(QStringLiteral("detail")).toString() == QLatin1String("GPL-3.0-only"));
    int lucide = -1;
    const QVariantMap icons = rowEndingWith(rows, "LICENSES/Lucide/LICENSE", &lucide);
    d.check(QStringLiteral("qp20: About lists Lucide's licence by name, once"),
            icons.value(QStringLiteral("title")).toString() == QLatin1String("Lucide icons")
                && countEndingWith(rows, "LICENSES/Lucide/LICENSE") == 1);
    if (lucide >= 0) {
        about->openNotice(lucide);
        d.check(QStringLiteral("qp20: and opens it: the ISC licence, with the MIT part for Feather's icons"),
                d.waitFor(3000, [&]() { return !about->noticeLoading(); })
                    && about->noticeText().startsWith(QLatin1String("ISC License"))
                    && about->noticeText().contains(QLatin1String("Feather"))
                    && about->noticeTitle() == QLatin1String("Lucide icons"));
    }

    // A folder as the .rpm installs it (/usr/share/licenses/dspi-uconsole)
    const QString root = QDir(d.outDir()).filePath(QStringLiteral("qp20-rpm-notices"));
    QDir(root).removeRecursively();
    const bool made = writeFile(root + QStringLiteral("/NOTICE.txt"), "Licence: GPL-3.0-only\n")
        && writeFile(root + QStringLiteral("/LICENSES/GPL-3.0.txt"), "GNU GENERAL PUBLIC LICENSE\n")
        && writeFile(root + QStringLiteral("/LICENSES/TERMS.txt"), "terms\n")
        && writeFile(root + QStringLiteral("/LICENSES/Lucide/LICENSE"), "ISC License\n")
        && writeFile(root + QStringLiteral("/LICENSES/AutoEq-MIT.txt"), "MIT\n");
    const QVariantList rpm = AboutController::noticesIn(root);
    d.check(QStringLiteral("qp20: an .rpm's folder lists the notice, the GPL with its label, the terms, Lucide and the rest"),
            made && rpm.size() == 5 && !rowEndingWith(rpm, "/NOTICE.txt").isEmpty()
                && rowEndingWith(rpm, "LICENSES/GPL-3.0.txt").value(QStringLiteral("detail")).toString() == QLatin1String("GPL-3.0-only")
                && rowEndingWith(rpm, "LICENSES/Lucide/LICENSE").value(QStringLiteral("title")).toString() == QLatin1String("Lucide icons")
                && countEndingWith(rpm, "LICENSES/Lucide/LICENSE") == 1
                && !rowEndingWith(rpm, "LICENSES/AutoEq-MIT.txt").isEmpty());
    QDir(root).removeRecursively();
}

const SmokeDriver::Section notices("qp20-notices", 45, &noticesSection);

} // namespace
