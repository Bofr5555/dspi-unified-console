#include "OnboardingText.h"
#include "Registrar.h"

#include <QByteArray>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

extern "C" {
#include "dspi_core.h"
}

// Not `onboarding`: main.qml's settings group has that id
static const Registrar::Controller<OnboardingText> onboardingRegistered("onboardingText");

namespace {
// `read` is a C function that replies with the bytes needed, including the NUL
QJsonDocument document(uint32_t (*read)(char *, uint32_t)) {
    const uint32_t needed = read(nullptr, 0);
    if (needed == 0) return {};
    QByteArray buf(int(needed), '\0');
    if (read(buf.data(), needed) != needed) return {};
    buf.chop(1);
    return QJsonDocument::fromJson(buf);
}
}

OnboardingText::OnboardingText(DSPiBridge *, QObject *parent) : QObject(parent) {}

QVariantList OnboardingText::releaseNotes() const {
    return document(dspi_onboarding_whats_new_json).array().toVariantList();
}

QVariantMap OnboardingText::gettingStarted() const {
    return document(dspi_onboarding_getting_started_json).object().toVariantMap();
}
