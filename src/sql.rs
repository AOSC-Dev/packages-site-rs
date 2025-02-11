pub const SQL_GET_PACKAGE_TESTING: &str = "
SELECT
    full_version,
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
SELECT name, dpkg_version, full_version FROM (SELECT
    p.name AS name,
    dpkg.dpkg_version dpkg_version,
    dpkg._vercomp dpkg_vercomp,
    pv.full_version full_version
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
    AND (coalesce(spabhost.value, '') = 'noarch') = (dpkg.architecture = 'noarch')) AS temp
WHERE
    dpkg_vercomp < comparable_dpkgver(full_version)
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
    coalesce(spsrc.key, '') raw_srctype,
    coalesce(spsrc.value, '') raw_srcurl,
    dpkg.dpkg_version dpkg_version,
    array_to_string(array_agg(DISTINCT dpkg.reponame), ',') dpkg_availrepos,
    coalesce(
        CASE
            WHEN dpkg.dpkg_version IS NOT null THEN (CASE WHEN
                dpkg._vercomp > comparable_dpkgver(p.full_version)
            THEN 1 ELSE 0 END) - (CASE WHEN
                dpkg._vercomp < comparable_dpkgver(p.full_version)
            THEN 1 ELSE 0 END)
            ELSE -1
        END,
        -2
    ) ver_compare
FROM
    v_packages p
    LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = p.name
    LEFT JOIN package_spec spsrc ON spsrc.package = p.name
    AND spsrc.key IN ('SRCTBL', 'GITSRC', 'SVNSRC', 'BZRSRC', 'SRCS')
GROUP BY
    name, tree, tree_category, p.branch, category,
    section, pkg_section, directory, description,
    version, full_version, commit_time, committer,
    dpkg.dpkg_version, dpkg._vercomp,
    spsrc.key, spsrc.value
ORDER BY
    name
";

pub const SQL_GET_PACKAGE_MISSING: &str = "
SELECT
    v_packages.name AS name,
    description,
    full_version,
    coalesce(dpkg_version, '') AS dpkg_version,
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
    AND (coalesce(spabhost.value, '') = 'noarch') = ($2 = 'noarch')
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
SELECT DISTINCT ON (package)
    package AS name,
    dpkg_version
FROM
    v_dpkg_packages_new
WHERE
    repo = $1
    AND package NOT IN (
        SELECT
            name
        FROM
            packages
    )
    AND package NOT IN (
        SELECT
            name || '-dbg' AS name
        FROM
            packages
)
";

pub const SQL_GET_PACKAGE_INFO_GHOST: &str = "
SELECT
    DISTINCT package AS name,
    '' tree,
    '' tree_category,
    '' branch,
    '' category,
    '' section,
    '' pkg_section,
    '' directory,
    '' description,
    '' AS version,
    '' full_version,
    to_timestamp(0) commit_time,
    '' committer,
    '' dependency,
    '' spec_path,
    false noarch,
    '' fail_arch,
    '' srctype,
    '' srcurl,
    false hasrevdep
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
            ts_headline('english', vp.description, to_tsquery($1)) desc_highlight,
            (
                CASE
                    WHEN vp.name = $2 THEN 1
                    WHEN position($3 in vp.name) = 0 THEN 3
                    ELSE 2
                END
            ) matchcls,
            ts_rank(to_tsvector('english', name || ' ' || description), to_tsquery($4)) ftrank
        FROM
            packages vp
        WHERE
            to_tsvector('english', name || ' ' || description) @@ to_tsquery($5)
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
        WHERE
            vp.name LIKE ('%' || $6 || '%')
            AND vp.name != $7
            AND NOT (to_tsvector('english', name || ' ' || description) @@ to_tsquery($8))
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
                dpkg._vercomp > comparable_dpkgver(full_version)
            THEN 1 ELSE 0 END) - (CASE WHEN
                dpkg._vercomp < comparable_dpkgver(full_version)
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
    full_version IS NOT NULL and dpkg_version IS NOT NULL
ORDER BY
    commit_time DESC,
    name ASC
LIMIT
    $1
";

pub const SQL_GET_PACKAGE_NEW: &str = "
SELECT
    DISTINCT ON (commit_time, name)
    name,
    description,
    full_version,
    commit_time,
    coalesce(
        CASE
            WHEN dpkg.dpkg_version IS NOT null THEN (CASE WHEN
                dpkg._vercomp > comparable_dpkgver(full_version)
            THEN 1 ELSE 0 END) - (CASE WHEN
                dpkg._vercomp < comparable_dpkgver(full_version)
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
    p.name AS name,
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
    AND (coalesce(spabhost.value, '') = 'noarch') = (dpkg.architecture = 'noarch')
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
    coalesce(dep.dependency, '') dependency,
    (coalesce(spabhost.value, '') = 'noarch') noarch,
    coalesce(spfailarch.value, '') fail_arch,
    coalesce(spsrc.key, '') srctype,
    coalesce(spsrc.value, '') srcurl,
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
    _vercomp DESC,
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
            WHEN path = '' OR path = '.' THEN ''
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
    path, name
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
                coalesce(spabhost.value, '') = 'noarch'
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
            coalesce(spabhost.value, '') = 'noarch'
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
                coalesce(spabhost.value, 'noarch') != 'noarch'
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
            coalesce(spabhost.value, 'noarch') != 'noarch'
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
    coalesce(relop, '') || coalesce(version, '') AS version,
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

pub const SQL_GET_PACKAGE_LIBRARY_DEP: &str = "
SELECT
    package
FROM
    v_so_breaks_dep
WHERE
    dep_package = $1
    AND package NOT LIKE '%+32'
    AND package NOT LIKE 'gcc+cross-%'
    AND package NOT IN ('cuda', 'latx', 'liblol', 'dropbox', 'intel-oneapi-basekit')
ORDER BY
    package
";

pub const SQL_GET_PACKAGE_SO_REVDEPS: &str = "
SELECT
    DISTINCT s.name, s.package
FROM
    pv_package_sodep s
INNER JOIN (
    SELECT
        p.package, p.name, p.ver
    FROM
        pv_package_sodep p
    INNER JOIN
        v_packages_new n
    ON
        p.package = n.package
        AND p.version = n.version
    WHERE
        p.package = $1
        AND p.depends = 0
        AND p.ver IS NOT NULL
) n
ON
    s.name = n.name
    AND s.ver = n.ver
    AND s.package <> n.package
WHERE
    s.depends = 1
ORDER BY
    s.name, s.package
";
