pub const SQL_GET_PACKAGE_TESTING: &str = "
SELECT
    version,
    spec_path,
    tree,
    branch
FROM
    package_testing
WHERE
    package = $1
";

pub const SQL_GET_PACKAGE_ERRORS: &str = "
SELECT
    message,
    path,
    tree,
    branch,
    line,
    col
FROM
    package_errors
WHERE
    package = $1
";

pub const SQL_GET_PACKAGE_CHANGELOG: &str = "
SELECT
    package,
    githash,
    version,
    tree,
    branch,
    urgency,
    message,
    maintainer_name,
    maintainer_name,
    maintainer_email,
    timestamp
FROM
    package_changes
WHERE
    package = $1
ORDER BY
    timestamp DESC
";

pub const SQL_GET_REPO_COUNT: &str = "
SELECT
    drs.repo AS name,
    dr.realname AS realname,
    dr.architecture,
    dr.suite AS branch,
    dr.date AS date,
    dr.testing AS testing,
    dr.category AS category,
    (drm.name IS NULL) AS testingonly,
    coalesce(drs.packagecnt, 0) AS pkgcount,
    coalesce(drs.ghostcnt, 0) AS ghost,
    coalesce(drs.laggingcnt, 0) AS lagging,
    coalesce(drs.missingcnt, 0) AS missing
FROM
    dpkg_repo_stats drs
    LEFT JOIN dpkg_repos dr ON dr.name = drs.repo
    LEFT JOIN (
        SELECT
            drs2.repo repo,
            drs2.packagecnt packagecnt
        FROM
            dpkg_repo_stats drs2
            LEFT JOIN dpkg_repos dr2 ON dr2.name = drs2.repo
        WHERE
            dr2.testing = 0
    ) drs_m ON drs_m.repo = dr.realname
    LEFT JOIN dpkg_repos drm ON drm.realname = dr.realname
    AND drm.testing = 0
ORDER BY
    drs_m.packagecnt DESC,
    dr.realname ASC,
    dr.testing ASC
";

pub const SQL_GET_TREES: &str = "
SELECT
    tree AS name,
    category,
    url,
    max(date) date,
    count(name) pkgcount
FROM
    (
        SELECT
            p.name,
            p.tree,
            t.category,
            t.url,
            p.commit_time date
        FROM
            v_packages p
            INNER JOIN trees t ON t.name = p.tree
    ) q1
GROUP BY
    tree, category, url
ORDER BY
    pkgcount DESC
";

pub const SQL_GET_PACKAGE_LAGGING: &str = "
SELECT
    p.name name,
    dpkg.dpkg_version dpkg_version,
    (
        (
            CASE
                WHEN coalesce(pv.epoch, '') = '' THEN ''
                ELSE pv.epoch || ':'
            END
        ) || pv.version || (
            CASE
                WHEN coalesce(pv.release, '') IN ('', '0') THEN ''
                ELSE '-' || pv.release
            END
        )
    ) full_version
FROM
    packages p
    LEFT JOIN package_spec spabhost ON spabhost.package = p.name
    AND spabhost.key = 'ABHOST'
    LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = p.name
    LEFT JOIN package_versions pv ON pv.package = p.name
    AND pv.branch = dpkg.branch
WHERE
    dpkg.repo = $1
    AND dpkg_version IS NOT null
    AND (
        dpkg.architecture = 'noarch'
        OR $2 != 'noarch'
    )
    AND (
        (spabhost.value = 'noarch') = (dpkg.architecture IS 'noarch')
    )
GROUP BY
    name
HAVING
    (
        comparable_dpkgver(max_dpkgver(dpkg_version)) < comparable_dpkgver(full_version)
    )
ORDER BY
    name
";

pub const SQL_GET_PACKAGE_LIST: &str = "
SELECT
    p.name,
    p.tree,
    p.tree_category,
    p.branch,
    p.category,
    p.section,
    p.pkg_section,
    p.directory,
    p.description,
    p.version,
    p.full_version,
    p.commit_time,
    p.committer,
    dpkg.dpkg_version dpkg_version,
    group_concat(DISTINCT dpkg.reponame) dpkg_availrepos,
    coalesce(
        CASE
            WHEN dpkg.dpkg_version IS NOT null THEN (CASE WHEN
                comparable_dpkgver(dpkg.dpkg_version) > comparable_dpkgver(p.full_version)
            THEN 1 ELSE 0 END) - (CASE WHEN
                comparable_dpkgver(dpkg.dpkg_version) < comparable_dpkgver(p.full_version)
            THEN 1 ELSE 0 END)
            ELSE -1
        END,
        -2
    ) ver_compare
FROM
    v_packages p
    LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = p.name
GROUP BY
    name
ORDER BY
    name
";

pub const SQL_GET_PACKAGE_MISSING: &str = "
SELECT
    v_packages.name name,
    description,
    full_version,
    dpkg_version,
    v_packages.tree_category
FROM
    v_packages
    LEFT JOIN package_spec spabhost ON spabhost.package = v_packages.name
    AND spabhost.key = 'ABHOST'
    LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = v_packages.name
    AND dpkg.reponame = $1
WHERE
    full_version IS NOT null
    AND dpkg_version IS null
    AND ((spabhost.value = 'noarch') = ($2 = 'noarch'))
    AND (
        EXISTS(
            SELECT
                1
            FROM
                dpkg_repos
            WHERE
                realname = $3
                AND category = 'bsp'
        ) = (v_packages.tree_category = 'bsp')
    )
ORDER BY
    name
";

pub const SQL_GET_PACKAGE_GHOST: &str = "
SELECT
    package name,
    dpkg_version
FROM
    v_dpkg_packages_new
WHERE
    repo = $1
    AND name NOT IN (
        SELECT
            name
        FROM
            packages
    )
    AND name NOT IN (
        SELECT
            name || '-dbg' name
        FROM
            packages
    )
GROUP BY
    name
";

pub const SQL_GET_PACKAGE_INFO_GHOST: &str = "
SELECT
    DISTINCT package name,
    '' tree,
    '' tree_category,
    '' branch,
    '' category,
    '' section,
    '' pkg_section,
    '' directory,
    '' description,
    '' version,
    '' full_version,
    NULL commit_time,
    '' committer,
    '' dependency,
    '' spec_path,
    0 noarch,
    NULL fail_arch,
    NULL srctype,
    NULL srcurl,
    0 hasrevdep
FROM
    dpkg_packages
WHERE
    package = $1
";

pub const SQL_SEARCH_PACKAGES_DESC: &str = "
SELECT
    q.name,
    q.description,
    q.desc_highlight,
    vp.full_version
FROM
    (
        SELECT
            vp.name,
            vp.description,
            highlight(fts_packages, 1, '<b>', '</b>') desc_highlight,
            (
                CASE
                    WHEN vp.name = $1 THEN 1
                    WHEN instr(vp.name, $2) = 0 THEN 3
                    ELSE 2
                END
            ) matchcls,
            bm25(fts_packages, 5, 1) ftrank
        FROM
            packages vp
            INNER JOIN fts_packages fp ON fp.name = vp.name
        WHERE
            fts_packages MATCH $3
        UNION
        ALL
        SELECT
            vp.name,
            vp.description,
            vp.description desc_highlight,
            2 matchcls,
            1.0 ftrank
        FROM
            v_packages vp
            LEFT JOIN fts_packages fp ON fp.name = vp.name
            AND fts_packages MATCH $4
        WHERE
            vp.name LIKE ('%' || $5 || '%')
            AND vp.name != $6
            AND fp.name IS NULL
    ) q
    INNER JOIN v_packages vp ON vp.name = q.name
ORDER BY
    q.matchcls,
    q.ftrank,
    vp.commit_time DESC,
    q.name
";

pub const SQL_GET_PACKAGE_NEW_LIST: &str = "
SELECT DISTINCT ON (commit_time, name)
    name,
    dpkg.dpkg_version dpkg_version,
    description,
    full_version,
    commit_time,
    coalesce(
        CASE
            WHEN dpkg_version IS NOT null THEN (CASE WHEN
                comparable_dpkgver(dpkg_version) > comparable_dpkgver(full_version)
            THEN 1 ELSE 0 END) - (CASE WHEN
                comparable_dpkgver(dpkg_version) < comparable_dpkgver(full_version)
            THEN 1 ELSE 0 END)
            ELSE -1
        END,
        -2
    ) ver_compare,
    CASE
        WHEN error.package IS NOT NULL THEN 1
        ELSE CASE
            WHEN testing.package IS NOT NULL THEN 2
            ELSE 0
        END
    END AS status
FROM
    v_packages
    LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = v_packages.name
    LEFT JOIN (
        SELECT
            DISTINCT package
        FROM
            package_testing
    ) testing ON testing.package = v_packages.name
    LEFT JOIN (
        SELECT
            DISTINCT package
        FROM
            package_errors
    ) error ON error.package = v_packages.name
WHERE
    full_version IS NOT null
ORDER BY
    commit_time DESC,
    name ASC
LIMIT
    $1
";

pub const SQL_GET_PACKAGE_NEW: &str = "
SELECT DISTINCT ON (commit_time, name)
    name,
    description,
    full_version,
    commit_time,
    coalesce(
        CASE
            WHEN dpkg.dpkg_version IS NOT null THEN (CASE WHEN
                comparable_dpkgver(dpkg.dpkg_version) > comparable_dpkgver(full_version)
            THEN 1 ELSE 0 END) - (CASE WHEN
                comparable_dpkgver(dpkg.dpkg_version) < comparable_dpkgver(full_version)
            THEN 1 ELSE 0 END)
            ELSE -1
        END,
        -2
    ) ver_compare,
    CASE
        WHEN error.package IS NOT NULL THEN 1
        ELSE CASE
            WHEN testing.package IS NOT NULL THEN 2
            ELSE 0
        END
    END AS status
FROM
    v_packages
    LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = v_packages.name
    LEFT JOIN (
        SELECT
            DISTINCT package
        FROM
            package_testing
    ) testing ON testing.package = v_packages.name
    LEFT JOIN (
        SELECT
            DISTINCT package
        FROM
            package_errors
    ) error ON error.package = v_packages.name
WHERE
    full_version IS NOT null
ORDER BY
    commit_time DESC,
    name ASC
LIMIT
    10
";

pub const SQL_GET_PACKAGE_REPO: &str = "
SELECT
    p.name name,
    p.full_version full_version,
    dpkg.dpkg_version dpkg_version,
    p.description description,
    CASE
        WHEN error.package IS NOT NULL THEN 1
        ELSE CASE
            WHEN testing.package IS NOT NULL THEN 2
            ELSE 0
        END
    END AS status
FROM
    v_packages p
    LEFT JOIN (
        SELECT
            DISTINCT package
        FROM
            package_testing
    ) testing ON testing.package = p.name
    LEFT JOIN (
        SELECT
            DISTINCT package
        FROM
            package_errors
    ) error ON error.package = p.name
    LEFT JOIN package_spec spabhost ON spabhost.package = p.name
    AND spabhost.key = 'ABHOST'
    LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = p.name
WHERE
    dpkg.repo = $1
    AND (
        (spabhost.value = 'noarch') = (dpkg.architecture = 'noarch')
    )
ORDER BY
    p.name
";

pub const SQL_GET_PACKAGE_INFO: &str = "
SELECT
    name,
    tree,
    tree_category,
    branch,
    category,
    section,
    pkg_section,
    directory,
    description,
    version,
    full_version,
    commit_time,
    committer,
    dep.dependency dependency,
    (coalesce(spabhost.value, '') = 'noarch') noarch,
    coalesce(spfailarch.value, '') fail_arch,
    spsrc.key srctype,
    spsrc.value srcurl,
    v_packages.spec_path spec_path,
    EXISTS(
        SELECT
            1
        FROM
            package_dependencies revpd
        WHERE
            revpd.relationship IN ('PKGDEP', 'BUILDDEP', 'PKGRECOM', 'PKGSUG')
            AND revpd.dependency = v_packages.name
    ) hasrevdep
FROM
    v_packages
    LEFT JOIN (
        SELECT
            package,
            string_agg(
                dependency || '|' || coalesce(relop, '') || coalesce(version, '') || '|' || relationship || '|' || architecture,
                ','
            ) dependency
        FROM
            package_dependencies
        GROUP BY
            package
    ) dep ON dep.package = v_packages.name
    LEFT JOIN package_spec spabhost ON spabhost.package = v_packages.name
    AND spabhost.key = 'ABHOST'
    LEFT JOIN package_spec spfailarch ON spfailarch.package = v_packages.name
    AND spfailarch.key = 'FAIL_ARCH'
    LEFT JOIN package_spec spsrc ON spsrc.package = v_packages.name
    AND spsrc.key IN ('SRCTBL', 'GITSRC', 'SVNSRC', 'BZRSRC', 'SRCS')
WHERE
    name = $1
";

pub const SQL_GET_PACKAGE_DPKG: &str = "
SELECT
    version,
    dp.architecture,
    repo,
    dr.realname reponame,
    dr.testing testing,
    filename,
    size
FROM
    dpkg_packages dp
    LEFT JOIN dpkg_repos dr ON dr.name = dp.repo
WHERE
    package = $1
ORDER BY
    dr.realname ASC,
    comparable_dpkgver(version) DESC,
    testing DESC
";

pub const SQL_GET_PACKAGE_VERSIONS: &str = "
SELECT
    v.branch,
    (
        (
            CASE
                WHEN coalesce(epoch, '') = '' THEN ''
                ELSE epoch || ':'
            END
        ) || version || (
            CASE
                WHEN coalesce(release, '') IN ('', '0') THEN ''
                ELSE '-' || release
            END
        )
    ) fullver
FROM
    package_versions v
    INNER JOIN packages p ON p.name = v.package
    INNER JOIN tree_branches b ON b.tree = p.tree
    AND b.branch = v.branch
WHERE
    v.package = $1
ORDER BY
    b.priority DESC
";

pub const SQL_GET_PACKAGE_DEB_LOCAL: &str = "
SELECT
    package,
    version,
    architecture,
    repo,
    maintainer,
    installed_size,
    filename,
    size,
    sha256
FROM
    dpkg_packages
WHERE
    package = $1
    AND version = $2
    AND repo = $3
";

pub const SQL_GET_PACKAGE_DEB_FILES: &str = r#"
SELECT
    (
        CASE
            WHEN path = '' THEN ''
            ELSE '/' || path
        END
    ) || '/' || "name" filename,
    "size",
    ftype,
    perm,
    uid,
    gid,
    uname,
    gname
FROM
    pv_package_files
WHERE
    package = $1
    AND version = $2
    AND repo = $3
    AND ftype != 5
ORDER BY
    filename
"#;

pub const SQL_GET_PACKAGE_SODEP: &str = "
SELECT
    depends,
    name || ver soname
FROM
    pv_package_sodep
WHERE
    package = $1
    AND version = $2
    AND repo = $3
ORDER BY
    depends,
    name,
    ver
";

pub const SQL_ISSUES_STATS: &str = "
SELECT
    q1.repo,
    q1.errno,
    q1.cnt,
    round(
        (q1.cnt :: float8 / coalesce(q2.total, s.cnt)) :: numeric,
        5
    ) :: float8 ratio
FROM
    (
        SELECT
            repo,
            errno,
            count(DISTINCT package) cnt
        FROM
            pv_package_issues
        GROUP BY
            GROUPING SETS ((repo, errno), ())
    ) q1
    LEFT JOIN (
        SELECT
            repo,
            count(package) cnt
        FROM
            v_packages_new
        GROUP BY
            repo
    ) s ON s.repo = q1.repo
    LEFT JOIN (
        SELECT
            b.name repo,
            count(DISTINCT p.name) total
        FROM
            package_versions v
            INNER JOIN packages p ON v.package = p.name
            INNER JOIN tree_branches b ON b.tree = p.tree
            AND b.branch = v.branch
        GROUP BY
            GROUPING SETS ((b.name), ())
    ) q2 ON q2.repo IS NOT DISTINCT
FROM
    q1.repo
";

pub const SQL_ISSUES_RECENT: &str = "
SELECT
    package,
    version,
    array_agg(
        DISTINCT errno
        ORDER BY
            errno
    ) errs
FROM
    pv_package_issues
WHERE
    errno != 311
GROUP BY
    package,
    version
ORDER BY
    max(mtime) DESC
LIMIT
    10
";

pub const SQL_ISSUES_CODE: &str = r#"
SELECT
    package "name",
    array_agg(DISTINCT "version") versions,
    array_agg(DISTINCT branch) branches,
    (array_agg(filename)) [1] filename,
    max(filecount) filecount
FROM
    (
        SELECT
            package,
            "version",
            substring(
                repo
                from
                    position('/' in repo) + 1
            ) branch,
            max("level") "level",
            (array_agg(filename)) [1] filename,
            count(filename) filecount
        FROM
            pv_package_issues
        WHERE
            errno = $1
            AND coalesce(repo = $2, TRUE)
        GROUP BY
            package,
            version,
            repo
    ) q1
GROUP BY
    package
ORDER BY
    package
"#;

pub const SQL_ISSUES_PACKAGE: &str = "
SELECT
    errno,
    version,
    repo,
    filecount,
    level,
    filename,
    detail
FROM
    (
        SELECT
            errno,
            version,
            repo,
            level,
            filename,
            detail,
            max(rowid) OVER (PARTITION BY errno, version, repo) filecount,
            rowid
        FROM
            (
                SELECT
                    errno,
                    version,
                    repo,
                    level,
                    filename,
                    detail,
                    count(*) OVER (PARTITION BY errno, version, repo) filecount,
                    row_number() OVER (
                        PARTITION BY errno,
                        version,
                        repo
                        ORDER BY
                            level,
                            filename
                    ) rowid
                FROM
                    pv_package_issues
                WHERE
                    package = $1
            ) q1
    ) q2
WHERE
    rowid <= 500
ORDER BY
    errno,
    version DESC,
    repo,
    level,
    filename
";

pub const SQL_GET_DEB_LIST_HASARCH: &str = "
SELECT
    dp.filename,
    rtrim(
        CASE
            WHEN dpnew.package IS NULL THEN 'old,'
            ELSE ''
        END || CASE
            WHEN packages.name IS NULL THEN 'outoftree,'
            ELSE ''
        END || CASE
            WHEN (
                spabhost.value = 'noarch'
                AND dpnoarch.package IS NULL
            ) THEN 'noarch'
            ELSE ''
        END,
        ','
    ) removereason
FROM
    dpkg_packages dp
    LEFT JOIN (
        SELECT
            package,
            max_dpkgver(version) version
        FROM
            dpkg_packages
        WHERE
            repo = $1
        GROUP BY
            package
    ) dpnew USING (package, version)
    LEFT JOIN packages ON packages.name = dp.package
    LEFT JOIN package_spec spabhost ON spabhost.package = dp.package
    AND spabhost.key = 'ABHOST'
    LEFT JOIN (
        SELECT
            dp.package,
            max_dpkgver(dp.version) version
        FROM
            dpkg_packages dp
            INNER JOIN dpkg_repos dr ON dr.name = dp.repo
        WHERE
            dr.architecture = 'noarch'
        GROUP BY
            dp.package
    ) dpnoarch ON dpnoarch.package = dp.package
    AND dpnoarch.version = dpnew.version
WHERE
    (
        dpnew.package IS NULL
        OR packages.name IS NULL
        OR (
            spabhost.value = 'noarch'
            AND dpnoarch.package IS NULL
        )
    )
    AND dp.repo = $2
UNION
ALL
SELECT
    filename,
    'dup' removereason
FROM
    dpkg_package_duplicate
WHERE
    repo = $3
ORDER BY
    filename
";

pub const SQL_GET_DEB_LIST_NOARCH: &str = "
SELECT
    dp.filename,
    rtrim(
        CASE
            WHEN dpnew.package IS NULL THEN 'old,'
            ELSE ''
        END || CASE
            WHEN packages.name IS NULL THEN 'outoftree,'
            ELSE ''
        END || CASE
            WHEN (
                spabhost.value != 'noarch'
                AND dphasarch.package IS NULL
            ) THEN 'hasarch'
            ELSE ''
        END,
        ','
    ) removereason
FROM
    dpkg_packages dp
    LEFT JOIN (
        SELECT
            package,
            max_dpkgver(version) version
        FROM
            dpkg_packages
        WHERE
            repo = $1
        GROUP BY
            package
    ) dpnew USING (package, version)
    LEFT JOIN packages ON packages.name = dp.package
    LEFT JOIN package_spec spabhost ON spabhost.package = dp.package
    AND spabhost.key = 'ABHOST'
    LEFT JOIN (
        SELECT
            dp.package,
            max_dpkgver(dp.version) version
        FROM
            dpkg_packages dp
            INNER JOIN dpkg_repos dr ON dr.name = dp.repo
        WHERE
            dr.architecture != 'noarch'
        GROUP BY
            dp.package
    ) dphasarch ON dphasarch.package = dp.package
    AND dphasarch.version = dpnew.version
WHERE
    (
        dpnew.package IS NULL
        OR packages.name IS NULL
        OR (
            spabhost.value != 'noarch'
            AND dphasarch.package IS NULL
        )
    )
    AND dp.repo = $2
UNION
ALL
SELECT
    filename,
    'dup' removereason
FROM
    dpkg_package_duplicate
WHERE
    repo = $3
ORDER BY
    filename
";

pub const SQL_GET_PACKAGE_REV_REL: &str = "
SELECT
    package,
    coalesce(relop, '') || coalesce(version, '') version,
    relationship,
    architecture
FROM
    package_dependencies
WHERE
    dependency = $1
    AND relationship IN ('PKGDEP', 'BUILDDEP', 'PKGRECOM', 'PKGSUG')
ORDER BY
    relationship,
    package,
    architecture
";
