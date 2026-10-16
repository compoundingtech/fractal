// FD-only macOS authority inspection. Kept behind the Mac-unavailable guard until
// the exact native fixture matrix is qualified. No ACLs or permissions are changed.
#include <sys/acl.h>
#include <sys/stat.h>
#include <membership.h>
#include <errno.h>
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

// The exported ACE namespace must match each public entry enumerator we use.
_Static_assert(ACL_EXTENDED_ALLOW == KAUTH_ACE_PERMIT, "allow tag namespace");
_Static_assert(ACL_EXTENDED_DENY == KAUTH_ACE_DENY, "deny tag namespace");
_Static_assert(ACL_ENTRY_INHERITED == KAUTH_ACE_INHERITED, "inherited flag namespace");
_Static_assert(ACL_ENTRY_FILE_INHERIT == KAUTH_ACE_FILE_INHERIT, "file inherit namespace");
_Static_assert(ACL_ENTRY_DIRECTORY_INHERIT == KAUTH_ACE_DIRECTORY_INHERIT, "directory inherit namespace");
_Static_assert(ACL_ENTRY_LIMIT_INHERIT == KAUTH_ACE_LIMIT_INHERIT, "limit inherit namespace");
_Static_assert(ACL_ENTRY_ONLY_INHERIT == KAUTH_ACE_ONLY_INHERIT, "inherit-only namespace");
_Static_assert(ACL_FLAG_NO_INHERIT == KAUTH_ACL_NO_INHERIT, "no inherit namespace");

struct st_acl_observation {
    uint32_t reason; // 0 accepted, 1 query, 2 representation, 3 unknown, 4 allow, 5 qualifier
    int32_t os_error;
    uint32_t entries;
    uint64_t observed_maximum;
    uint64_t expected_maximum;
};

// -Werror=switch-enum makes an SDK enum extension a build failure. Aliases have
// identical values and are covered by their file-right counterparts.
static int known_permission(acl_perm_t p) {
    switch (p) {
    case ACL_READ_DATA: case ACL_WRITE_DATA: case ACL_EXECUTE: case ACL_DELETE:
    case ACL_APPEND_DATA: case ACL_DELETE_CHILD: case ACL_READ_ATTRIBUTES:
    case ACL_WRITE_ATTRIBUTES: case ACL_READ_EXTATTRIBUTES:
    case ACL_WRITE_EXTATTRIBUTES: case ACL_READ_SECURITY: case ACL_WRITE_SECURITY:
    case ACL_CHANGE_OWNER: case ACL_SYNCHRONIZE: return 1;
    }
    return 0;
}
static int known_flag(acl_flag_t f) {
    switch (f) {
    case ACL_FLAG_DEFER_INHERIT: case ACL_FLAG_NO_INHERIT:
    case ACL_ENTRY_INHERITED: case ACL_ENTRY_FILE_INHERIT:
    case ACL_ENTRY_DIRECTORY_INHERIT: case ACL_ENTRY_LIMIT_INHERIT:
    case ACL_ENTRY_ONLY_INHERIT: return 1;
    }
    return 0;
}
static int known_tag(acl_tag_t t) {
    switch (t) {
    case ACL_UNDEFINED_TAG: return 0;
    case ACL_EXTENDED_ALLOW: case ACL_EXTENDED_DENY: return 1;
    }
    return 0;
}

// Query errors are errors even when errno happens to be ENOENT. Absence is
// accepted only from a successful property-presence query after successful fstatx.
uint32_t st_client_mode_acl_query_result(int result, int saved_errno,
                                       struct st_acl_observation *out) {
    if (result == 0) return 0;
    out->reason = 1;
    out->os_error = saved_errno;
    return 1;
}

// Pure validation of the public exported representation. Arbitrary alignment is
// supported; tests pass mutated byte buffers through this same production parser.
uint32_t st_client_mode_acl_check_export(const void *bytes, size_t size) {
    struct kauth_filesec header = {0};
    const uint32_t known_rights = ACL_READ_DATA | ACL_WRITE_DATA | ACL_EXECUTE |
        ACL_DELETE | ACL_APPEND_DATA | ACL_DELETE_CHILD | ACL_READ_ATTRIBUTES |
        ACL_WRITE_ATTRIBUTES | ACL_READ_EXTATTRIBUTES | ACL_WRITE_EXTATTRIBUTES |
        ACL_READ_SECURITY | ACL_WRITE_SECURITY | ACL_CHANGE_OWNER | ACL_SYNCHRONIZE;
    const uint32_t known_flags = KAUTH_ACE_INHERITED | KAUTH_ACE_FILE_INHERIT |
        KAUTH_ACE_DIRECTORY_INHERIT | KAUTH_ACE_LIMIT_INHERIT | KAUTH_ACE_ONLY_INHERIT;
    if (bytes == NULL || !KAUTH_FILESEC_VALID(size) ||
        size > KAUTH_FILESEC_SIZE(ACL_MAX_ENTRIES)) return 2;
    memcpy(&header, bytes, size < sizeof(header) ? size : sizeof(header));
    if (header.fsec_magic != KAUTH_FILESEC_MAGIC ||
        header.fsec_entrycount > ACL_MAX_ENTRIES ||
        KAUTH_FILESEC_SIZE(header.fsec_entrycount) != size) return 2;
    // Deferred inheritance (KAUTH bit16) and reserved raw low bits refuse.
    if ((header.fsec_flags & ~KAUTH_ACL_NO_INHERIT) != 0) return 3;
    for (uint32_t i = 0; i < header.fsec_entrycount; ++i) {
        struct kauth_ace entry;
        memcpy(&entry, (const unsigned char *)bytes + KAUTH_FILESEC_SIZE(i), sizeof(entry));
        uint32_t tag = entry.ace_flags & KAUTH_ACE_KINDMASK;
        if ((tag != ACL_EXTENDED_ALLOW && tag != ACL_EXTENDED_DENY) ||
            (entry.ace_flags & ~(KAUTH_ACE_KINDMASK | known_flags)) != 0 ||
            (entry.ace_rights & ~known_rights) != 0) return 3;
    }
    return 0;
}

// Independent of OS queries: tests can inject accessor/export disagreement.
uint32_t st_client_mode_acl_check_entry(uint32_t exported_flags,
                                      uint64_t exported_rights,
                                      int accessor_tag, uint64_t accessor_rights) {
    if (!known_tag((acl_tag_t)accessor_tag) ||
        (exported_flags & KAUTH_ACE_KINDMASK) != (uint32_t)accessor_tag ||
        exported_rights != accessor_rights) return 3;
    return 0;
}

// Read-only SDK layout facts for test construction, not a production override.
// Rust declares this symbol only under cfg(test).
void st_client_mode_acl_export_layout(size_t *out) {
    out[0] = KAUTH_FILESEC_SIZE(1);
    out[1] = offsetof(struct kauth_filesec, fsec_magic);
    out[2] = offsetof(struct kauth_filesec, fsec_acl.acl_entrycount);
    out[3] = offsetof(struct kauth_filesec, fsec_acl.acl_flags);
    out[4] = KAUTH_FILESEC_SIZE(0) + offsetof(struct kauth_ace, ace_flags);
    out[5] = KAUTH_FILESEC_SIZE(0) + offsetof(struct kauth_ace, ace_rights);
    out[6] = KAUTH_FILESEC_MAGIC;
}

void st_client_mode_acl_inspect(int fd, int private_object,
                               struct st_acl_observation *out) {
    const uint64_t rights = ACL_READ_DATA | ACL_WRITE_DATA | ACL_EXECUTE |
        ACL_DELETE | ACL_APPEND_DATA | ACL_DELETE_CHILD | ACL_READ_ATTRIBUTES |
        ACL_WRITE_ATTRIBUTES | ACL_READ_EXTATTRIBUTES | ACL_WRITE_EXTATTRIBUTES |
        ACL_READ_SECURITY | ACL_WRITE_SECURITY | ACL_CHANGE_OWNER | ACL_SYNCHRONIZE;
    const uint64_t mutation = ACL_WRITE_DATA | ACL_DELETE | ACL_APPEND_DATA |
        ACL_DELETE_CHILD | ACL_WRITE_ATTRIBUTES | ACL_WRITE_EXTATTRIBUTES |
        ACL_WRITE_SECURITY | ACL_CHANGE_OWNER;
    const uint32_t entry_flags = KAUTH_ACE_INHERITED | KAUTH_ACE_FILE_INHERIT |
        KAUTH_ACE_DIRECTORY_INHERIT | KAUTH_ACE_LIMIT_INHERIT | KAUTH_ACE_ONLY_INHERIT;
    // Raw exported flags use KAUTH, not the public acl_flag_t namespace.
    // Deferred inheritance could change the ACL at rename; explicitly unsupported.
    const uint32_t acl_flags = KAUTH_ACL_NO_INHERIT;
    acl_t acl = NULL;
    filesec_t security = NULL;
    struct stat metadata;
    int acl_present = 0;
    struct kauth_filesec *external = NULL;
    uint64_t maximum = 0;
    memset(out, 0, sizeof(*out));
    out->expected_maximum = rights;
    // Exercise the exhaustive functions, and check the runtime library's full mask.
    if (!known_permission(ACL_READ_DATA) || !known_flag(ACL_ENTRY_INHERITED) ||
        acl_maximal_permset_mask_np(&maximum) != 0 ||
        ((out->observed_maximum = maximum), maximum != rights)) {
        out->reason = 3; goto done;
    }
    security = filesec_init();
    if (security == NULL) { out->reason = 1; out->os_error = errno; goto done; }
    int queried = fstatx_np(fd, &metadata, security);
    int query_errno = errno;
    if (st_client_mode_acl_query_result(queried, query_errno, out) != 0) goto done;
    if (filesec_query_property(security, FILESEC_ACL, &acl_present) != 0) {
        out->reason = 1; out->os_error = errno; goto done;
    }
    if (acl_present == 0) goto done;
    // filesec_query_property returns a nonzero validity bit, not necessarily 1.
    if (filesec_get_property(security, FILESEC_ACL, &acl) != 0 || acl == NULL) {
        out->reason = 1; out->os_error = errno; goto done;
    }
    ssize_t size = acl_size(acl);
    if (size < 0 || !KAUTH_FILESEC_VALID((size_t)size) ||
        (size_t)size > KAUTH_FILESEC_SIZE(ACL_MAX_ENTRIES)) {
        out->reason = 2; goto done;
    }
    external = malloc((size_t)size);
    if (external == NULL) { out->reason = 1; out->os_error = ENOMEM; goto done; }
    if (acl_copy_ext_native(external, acl, size) != size ||
        external->fsec_magic != KAUTH_FILESEC_MAGIC ||
        external->fsec_entrycount > ACL_MAX_ENTRIES ||
        KAUTH_FILESEC_SIZE(external->fsec_entrycount) != (size_t)size) {
        out->reason = 2; goto done;
    }
    uint32_t parsed = st_client_mode_acl_check_export(external, (size_t)size);
    if (parsed != 0) { out->reason = parsed; goto done; }
    if ((external->fsec_flags & ~acl_flags) != 0) { out->reason = 3; goto done; }
    out->entries = external->fsec_entrycount;
    for (uint32_t i = 0; i < out->entries; ++i) {
        acl_entry_t entry;
        acl_tag_t tag;
        acl_permset_mask_t mask;
        // Darwin returns 0 on success, not POSIX 1. The copied, SDK-defined
        // entry count distinguishes end-of-list from a query error.
        if (acl_get_entry(acl, i == 0 ? ACL_FIRST_ENTRY : ACL_NEXT_ENTRY, &entry) != 0 ||
            acl_get_tag_type(entry, &tag) != 0 ||
            acl_get_permset_mask_np(entry, &mask) != 0) {
            out->reason = 1; out->os_error = errno; goto done;
        }
        struct kauth_ace *raw = &external->fsec_ace[i];
        if (st_client_mode_acl_check_entry(raw->ace_flags, raw->ace_rights, tag, mask) != 0 ||
            (mask & ~rights) != 0 ||
            (raw->ace_flags & ~(KAUTH_ACE_KINDMASK | entry_flags)) != 0 ||
            (raw->ace_flags & KAUTH_ACE_KINDMASK) != (uint32_t)tag ||
            raw->ace_rights != mask) { out->reason = 3; goto done; }
        if (tag == ACL_EXTENDED_ALLOW) {
            void *qualifier = acl_get_qualifier(entry);
            id_t id;
            int id_type;
            if (qualifier == NULL) { out->reason = 5; out->os_error = errno; goto done; }
            int resolved = mbr_uuid_to_id(qualifier, &id, &id_type);
            acl_free(qualifier);
            if (resolved != 0) { out->reason = 5; out->os_error = resolved; goto done; }
            // Never infer effective access by ordering deny and allow entries.
            // Inherit-only and inherited grants receive the same refusal.
            if (private_object || (mask & mutation) != 0) { out->reason = 4; goto done; }
        }
    }
done:
    free(external);
    if (security != NULL) filesec_free(security);
    if (acl != NULL) acl_free(acl);
}
