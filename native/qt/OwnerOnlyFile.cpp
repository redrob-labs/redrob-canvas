// SPDX-License-Identifier: GPL-3.0-or-later
#include "OwnerOnlyFile.h"

#include <QByteArray>
#include <QDir>
#include <QFile>
#include <QFileInfo>

#ifdef Q_OS_WIN
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <aclapi.h>

#include <string>

namespace {

// The SID of the user this process runs as, copied out of the token.
QByteArray currentUserSid()
{
    HANDLE token = nullptr;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token))
        return {};
    DWORD size = 0;
    GetTokenInformation(token, TokenUser, nullptr, 0, &size);
    QByteArray buffer(int(size), '\0');
    const bool ok = size != 0 && GetTokenInformation(token, TokenUser, buffer.data(), size, &size);
    CloseHandle(token);
    if (!ok)
        return {};
    const auto *user = reinterpret_cast<const TOKEN_USER *>(buffer.constData());
    if (!IsValidSid(user->User.Sid))
        return {};
    const DWORD length = GetLengthSid(user->User.Sid);
    return QByteArray(reinterpret_cast<const char *>(user->User.Sid), int(length));
}

std::wstring nativePath(const QString &path)
{
    return QDir::toNativeSeparators(QFileInfo(path).absoluteFilePath()).toStdWString();
}

} // namespace

bool OwnerOnlyFile::restrictToOwner(const QString &path)
{
    QByteArray sid = currentUserSid();
    if (sid.isEmpty())
        return false;
    EXPLICIT_ACCESS_W access{};
    access.grfAccessPermissions = GENERIC_ALL;
    access.grfAccessMode = SET_ACCESS;
    access.grfInheritance = NO_INHERITANCE;
    access.Trustee.TrusteeForm = TRUSTEE_IS_SID;
    access.Trustee.TrusteeType = TRUSTEE_IS_USER;
    access.Trustee.ptstrName = reinterpret_cast<LPWSTR>(sid.data());
    PACL acl = nullptr;
    if (SetEntriesInAclW(1, &access, nullptr, &acl) != ERROR_SUCCESS)
        return false;
    std::wstring native = nativePath(path);
    const DWORD result = SetNamedSecurityInfoW(
        native.data(), SE_FILE_OBJECT,
        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, nullptr, nullptr, acl,
        nullptr);
    LocalFree(acl);
    return result == ERROR_SUCCESS;
}

bool OwnerOnlyFile::isOwnerOnly(const QString &path)
{
    QByteArray sid = currentUserSid();
    if (sid.isEmpty() || !QFile::exists(path))
        return false;
    PACL dacl = nullptr;
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    const std::wstring native = nativePath(path);
    if (GetNamedSecurityInfoW(native.c_str(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, nullptr,
                              nullptr, &dacl, nullptr, &descriptor)
        != ERROR_SUCCESS)
        return false;
    // A null DACL grants everyone full access.
    bool ownerOnly = dacl != nullptr;
    ACL_SIZE_INFORMATION info{};
    if (ownerOnly && !GetAclInformation(dacl, &info, sizeof info, AclSizeInformation))
        ownerOnly = false;
    for (DWORD i = 0; ownerOnly && i < info.AceCount; ++i) {
        void *ace = nullptr;
        if (!GetAce(dacl, i, &ace)) {
            ownerOnly = false;
            break;
        }
        const auto *header = static_cast<const ACE_HEADER *>(ace);
        if (header->AceType == ACCESS_DENIED_ACE_TYPE)
            continue; // a deny entry only narrows access
        if (header->AceType != ACCESS_ALLOWED_ACE_TYPE) {
            ownerOnly = false; // object or callback entries: not something we wrote
            break;
        }
        auto *allowed = static_cast<ACCESS_ALLOWED_ACE *>(ace);
        if (!EqualSid(reinterpret_cast<PSID>(&allowed->SidStart), sid.data()))
            ownerOnly = false;
    }
    LocalFree(descriptor);
    return ownerOnly;
}

#else

bool OwnerOnlyFile::restrictToOwner(const QString &path)
{
    return QFile::setPermissions(path, QFileDevice::ReadOwner | QFileDevice::WriteOwner);
}

bool OwnerOnlyFile::isOwnerOnly(const QString &path)
{
    const QFile file(path);
    // No group or other bit at all.
    const QFile::Permissions others = QFileDevice::ReadGroup | QFileDevice::WriteGroup
        | QFileDevice::ExeGroup | QFileDevice::ReadOther | QFileDevice::WriteOther
        | QFileDevice::ExeOther;
    return file.exists() && (file.permissions() & others) == 0;
}

#endif
