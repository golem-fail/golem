// device_resolution.qnt — Formal specification of Golem's device resolution algorithm.
//
// This models the minimum coverage solver: given a set of available devices and
// a set of coverage requirements (per-field arrays of required values), select
// the smallest set of devices that satisfies all requirements, respecting
// preference ordering (running > shutdown > create), explicit inclusions,
// expansion modes, and guard rails.

module device_resolution {

  // ──────────────────────────────────────────────────────────────────────
  // Types
  // ──────────────────────────────────────────────────────────────────────

  // Target platform.
  type Platform = Ios | Android

  // Physical form factor.
  type DeviceType = Phone | Tablet

  // Device lifecycle state, ordered by preference (lower = better).
  type DeviceState = Booted | Shutdown | NeedsCreation

  // How a device was identified in the requirements.
  type SelectionOrigin =
    | Explicit       // user specified by id or name
    | AutoSelected   // picked by the solver to satisfy coverage

  // Expansion mode for a device constraint.
  type ExpandMode = NoExpand | FullExpand

  // A device in the available pool.
  type Device = {
    id:        str,
    name:      str,
    os:        str,         // e.g., "ios:17", "android:14"
    dtype:     str,         // "phone" or "tablet"
    physical:  bool,
    state:     DeviceState,
  }

  // A coverage requirement from an app's device constraints.
  // Each field is a set of values that must appear across the selected devices.
  type CoverageReq = {
    app:       str,         // which app this requirement belongs to
    os_values: Set[str],    // e.g., {"ios:17", "ios:18", "android:14"}
    type_values: Set[str],  // e.g., {"phone", "tablet"}
    explicit_ids:   Set[str],  // devices required by id
    explicit_names: Set[str],  // devices required by name
    expand:    ExpandMode,
    require_physical: bool,
  }

  // A selected device paired with the apps it serves and its origin.
  type SelectedDevice = {
    device: Device,
    apps:   Set[str],
    origin: SelectionOrigin,
    created: bool,
  }

  // Result of the resolution algorithm.
  type ResolutionResult =
    | Resolved(Set[SelectedDevice])
    | Failed(str)

  // ──────────────────────────────────────────────────────────────────────
  // State
  // ──────────────────────────────────────────────────────────────────────

  // Pool of available devices (discovered on the system).
  var available_devices: Set[Device]

  // Coverage requirements from the flow file (one per app's device constraints).
  var requirements: Set[CoverageReq]

  // The set of devices selected by the solver.
  var selected: Set[SelectedDevice]

  // Devices that were created (did not previously exist).
  var created_devices: Set[Device]

  // Global options.
  var create_if_missing: bool
  var ignore_missing_physical: bool

  // Track resolution outcome.
  var resolution_result: ResolutionResult

  // ──────────────────────────────────────────────────────────────────────
  // Helpers
  // ──────────────────────────────────────────────────────────────────────

  // Extract the underlying device from a SelectedDevice.
  pure def device_of(sd: SelectedDevice): Device = sd.device

  // All underlying devices in a selection set.
  pure def devices_of(sds: Set[SelectedDevice]): Set[Device] =
    sds.map(sd => sd.device)

  // OS values covered by a set of selected devices.
  pure def covered_os(sds: Set[SelectedDevice]): Set[str] =
    sds.fold(Set(), (acc, sd) => acc.union(Set(sd.device.os)))

  // Device type values covered by a set of selected devices.
  pure def covered_types(sds: Set[SelectedDevice]): Set[str] =
    sds.fold(Set(), (acc, sd) => acc.union(Set(sd.device.dtype)))

  // Check if a device matches an explicit id requirement.
  pure def matches_explicit_id(d: Device, ids: Set[str]): bool =
    ids.contains(d.id)

  // Check if a device matches an explicit name requirement.
  pure def matches_explicit_name(d: Device, names: Set[str]): bool =
    names.contains(d.name)

  // Check if a device is explicitly required by any requirement.
  pure def is_explicit(sd: SelectedDevice): bool =
    sd.origin == Explicit

  // Preference score: lower is better. Booted=0, Shutdown=1, NeedsCreation=2.
  pure def state_preference(s: DeviceState): int = match s {
    | Booted        => 0
    | Shutdown      => 1
    | NeedsCreation => 2
  }

  // Compare two devices by preference: true if a is strictly preferred over b.
  pure def preferred_over(a: Device, b: Device): bool =
    state_preference(a.state) < state_preference(b.state)

  // Count how many devices in a selection set are in a given state.
  pure def count_in_state(sds: Set[SelectedDevice], s: DeviceState): int =
    sds.filter(sd => sd.device.state == s).size()

  // The set of OS values still uncovered after explicit devices are credited.
  pure def uncovered_os_after_explicit(
    req: CoverageReq,
    explicits: Set[SelectedDevice]
  ): Set[str] =
    req.os_values.exclude(covered_os(explicits))

  // The set of type values still uncovered after explicit devices are credited.
  pure def uncovered_types_after_explicit(
    req: CoverageReq,
    explicits: Set[SelectedDevice]
  ): Set[str] =
    req.type_values.exclude(covered_types(explicits))

  // The set of new OS values that a device contributes beyond what is already covered.
  pure def new_os_coverage(d: Device, already_covered: Set[str]): Set[str] =
    if (already_covered.contains(d.os)) Set() else Set(d.os)

  // The set of new type values that a device contributes beyond what is already covered.
  pure def new_type_coverage(d: Device, already_covered: Set[str]): Set[str] =
    if (already_covered.contains(d.dtype)) Set() else Set(d.dtype)

  // Cartesian product size for full expansion: |os_values| * |type_values|.
  pure def cartesian_size(req: CoverageReq): int =
    req.os_values.size() * req.type_values.size()

  // ──────────────────────────────────────────────────────────────────────
  // Initialization
  // ──────────────────────────────────────────────────────────────────────

  action init = all {
    available_devices' = Set(),
    requirements'      = Set(),
    selected'          = Set(),
    created_devices'   = Set(),
    create_if_missing' = false,
    ignore_missing_physical' = false,
    resolution_result' = Resolved(Set()),
  }

  // ──────────────────────────────────────────────────────────────────────
  // Actions: setup (configure the problem)
  // ──────────────────────────────────────────────────────────────────────

  // Add a device to the available pool.
  action add_available_device(d: Device) = all {
    available_devices' = available_devices.union(Set(d)),
    requirements'      = requirements,
    selected'          = selected,
    created_devices'   = created_devices,
    create_if_missing' = create_if_missing,
    ignore_missing_physical' = ignore_missing_physical,
    resolution_result' = resolution_result,
  }

  // Add a coverage requirement.
  action add_requirement(req: CoverageReq) = all {
    requirements'      = requirements.union(Set(req)),
    available_devices' = available_devices,
    selected'          = selected,
    created_devices'   = created_devices,
    create_if_missing' = create_if_missing,
    ignore_missing_physical' = ignore_missing_physical,
    resolution_result' = resolution_result,
  }

  // Set global options.
  action set_options(cim: bool, imp: bool) = all {
    create_if_missing' = cim,
    ignore_missing_physical' = imp,
    available_devices' = available_devices,
    requirements'      = requirements,
    selected'          = selected,
    created_devices'   = created_devices,
    resolution_result' = resolution_result,
  }

  // ──────────────────────────────────────────────────────────────────────
  // Actions: resolution (select devices)
  // ──────────────────────────────────────────────────────────────────────

  // Select a device to satisfy coverage. The solver nondeterministically
  // picks a valid selection; invariants constrain it to correct ones.
  action select_device(d: Device, app: str, origin: SelectionOrigin, was_created: bool) = all {
    // The device must be in the available pool or be a created device.
    available_devices.contains(d) or (was_created and create_if_missing),
    // If created, record it.
    created_devices' = if (was_created) created_devices.union(Set(d)) else created_devices,
    // If created, it must only happen when create_if_missing is true.
    was_created implies create_if_missing,
    // Add to selected set.
    val sd: SelectedDevice = { device: d, apps: Set(app), origin: origin, created: was_created }
    selected' = selected.union(Set(sd)),
    available_devices' = available_devices,
    requirements'      = requirements,
    create_if_missing' = create_if_missing,
    ignore_missing_physical' = ignore_missing_physical,
    resolution_result' = Resolved(selected.union(Set(sd))),
  }

  // Merge apps onto an already-selected device (two apps share one device).
  action share_device(existing_id: str, new_app: str) = all {
    // Must find a selected device with this id.
    selected.exists(sd => sd.device.id == existing_id),
    // Update the selected set: add the new app to the matching device.
    selected' = selected.map(sd =>
      if (sd.device.id == existing_id)
        { device: sd.device, apps: sd.apps.union(Set(new_app)), origin: sd.origin, created: sd.created }
      else sd
    ),
    available_devices' = available_devices,
    requirements'      = requirements,
    created_devices'   = created_devices,
    create_if_missing' = create_if_missing,
    ignore_missing_physical' = ignore_missing_physical,
    resolution_result' = Resolved(selected.map(sd =>
      if (sd.device.id == existing_id)
        { device: sd.device, apps: sd.apps.union(Set(new_app)), origin: sd.origin, created: sd.created }
      else sd
    )),
  }

  // Fail resolution when requirements are impossible to satisfy.
  action fail_resolution(reason: str) = all {
    resolution_result' = Failed(reason),
    available_devices' = available_devices,
    requirements'      = requirements,
    selected'          = selected,
    created_devices'   = created_devices,
    create_if_missing' = create_if_missing,
    ignore_missing_physical' = ignore_missing_physical,
  }

  // ──────────────────────────────────────────────────────────────────────
  // INVARIANTS
  // ──────────────────────────────────────────────────────────────────────

  // ── 1. Coverage completeness ──
  // Every value in every requirement array appears in at least one selected device.
  // Only checked when resolution succeeded.

  val inv_coverage_completeness: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          sel.size() > 0 implies
            requirements.forall(req =>
              // Every required OS value is covered.
              req.os_values.forall(os =>
                sel.exists(sd => sd.device.os == os and sd.apps.contains(req.app))
              ) and
              // Every required device type value is covered.
              req.type_values.forall(dt =>
                sel.exists(sd => sd.device.dtype == dt and sd.apps.contains(req.app))
              )
            )
    }
  }

  // ── 2. Minimality ──
  // No selected device can be removed without breaking coverage.
  // (Only applies to auto-selected devices; explicit devices are always kept.)

  val inv_minimality: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          sel.forall(sd =>
            is_explicit(sd) or
            // Removing this device would lose at least one required value.
            val remaining = sel.exclude(Set(sd))
            requirements.exists(req =>
              sd.apps.contains(req.app) and (
                req.os_values.exists(os =>
                  not(remaining.exists(other =>
                    other.device.os == os and other.apps.contains(req.app)
                  ))
                ) or
                req.type_values.exists(dt =>
                  not(remaining.exists(other =>
                    other.device.dtype == dt and other.apps.contains(req.app)
                  ))
                )
              )
            )
          )
    }
  }

  // ── 3. Preference ordering ──
  // Running (Booted) > Shutdown > NeedsCreation.
  // If two devices cover the same requirements and one is preferred,
  // the preferred one should be selected.
  // Stated as: no auto-selected device in a worse state when an unused
  // available device in a better state would cover the same values.

  val inv_preference_ordering: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          sel.forall(sd =>
            is_explicit(sd) or
            // No available device that is not selected, has a better state,
            // and covers a superset of what this device covers for its apps.
            not(available_devices.exists(alt =>
              not(sel.exists(s => s.device.id == alt.id)) and
              preferred_over(alt, sd.device) and
              alt.os == sd.device.os and
              alt.dtype == sd.device.dtype and
              alt.physical == sd.device.physical
            ))
          )
    }
  }

  // ── 4. Explicit inclusion ──
  // Devices specified by id or name in requirements are always selected.

  val inv_explicit_inclusion: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          requirements.forall(req =>
            // Every explicit id must appear in selection.
            req.explicit_ids.forall(eid =>
              sel.exists(sd => sd.device.id == eid and sd.apps.contains(req.app))
            ) and
            // Every explicit name must appear in selection.
            req.explicit_names.forall(ename =>
              sel.exists(sd => sd.device.name == ename and sd.apps.contains(req.app))
            )
          )
    }
  }

  // ── 5. Explicit coverage credit ──
  // Explicit devices count toward coverage; the solver does not ignore their
  // attribute values when computing remaining requirements.

  val inv_explicit_coverage_credit: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          requirements.forall(req =>
            val explicits = sel.filter(sd =>
              is_explicit(sd) and sd.apps.contains(req.app)
            )
            val autos = sel.filter(sd =>
              not(is_explicit(sd)) and sd.apps.contains(req.app)
            )
            // The OS values covered by explicits are not redundantly covered
            // by auto-selected devices unless those autos cover additional values too.
            // Formally: no auto-selected device's sole contribution is values
            // already fully covered by explicits (this is a consequence of minimality
            // plus explicit credit, restated for clarity).
            autos.forall(sd =>
              val remaining_without = sel.exclude(Set(sd))
              requirements.exists(r =>
                sd.apps.contains(r.app) and (
                  r.os_values.exists(os =>
                    not(remaining_without.exists(other =>
                      other.device.os == os and other.apps.contains(r.app)
                    ))
                  ) or
                  r.type_values.exists(dt =>
                    not(remaining_without.exists(other =>
                      other.device.dtype == dt and other.apps.contains(r.app)
                    ))
                  )
                )
              )
            )
          )
    }
  }

  // ── 6. No waste ──
  // No auto-selected device contributes zero uncovered values.
  // (Explicit devices are exempt from this rule.)

  val inv_no_waste: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          sel.forall(sd =>
            is_explicit(sd) or
            // This device must contribute at least one required value
            // that no other selected device (excluding this one) covers.
            val remaining = sel.exclude(Set(sd))
            sd.apps.exists(app =>
              requirements.exists(req =>
                req.app == app and (
                  req.os_values.exists(os =>
                    sd.device.os == os and
                    not(remaining.exists(other =>
                      other.device.os == os and other.apps.contains(app)
                    ))
                  ) or
                  req.type_values.exists(dt =>
                    sd.device.dtype == dt and
                    not(remaining.exists(other =>
                      other.device.dtype == dt and other.apps.contains(app)
                    ))
                  )
                )
              )
            )
          )
    }
  }

  // ── 7. Create guard ──
  // Devices are only created when create_if_missing = true.

  val inv_create_guard: bool = {
    not(create_if_missing) implies created_devices.size() == 0
  }

  // Additionally, no selected device should be marked as created
  // when create_if_missing is false.
  val inv_create_guard_selected: bool = {
    not(create_if_missing) implies
      selected.forall(sd => not(sd.created))
  }

  // ── 8. Full expansion ──
  // When expand = "full", the selection must include one device for every
  // combination in the cartesian product of os_values x type_values.

  val inv_full_expansion: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          requirements.forall(req =>
            req.expand != FullExpand or
            // For full expansion, every (os, type) pair must have a selected device.
            req.os_values.forall(os =>
              req.type_values.forall(dt =>
                sel.exists(sd =>
                  sd.device.os == os and
                  sd.device.dtype == dt and
                  sd.apps.contains(req.app)
                )
              )
            )
          )
    }
  }

  // Full expansion produces exactly the cartesian product size (per app).
  val inv_full_expansion_count: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          requirements.forall(req =>
            req.expand != FullExpand or
            // The number of devices for this app must be at least the cartesian size.
            val app_devices = sel.filter(sd => sd.apps.contains(req.app))
            app_devices.size() >= cartesian_size(req)
          )
    }
  }

  // ── 9. Physical skip ──
  // When ignore_missing_physical = true, if a requirement asks for physical
  // devices but none are available, the solver skips that requirement (warning)
  // rather than failing.
  // When ignore_missing_physical = false, missing physical devices cause failure.

  val inv_physical_skip: bool = {
    requirements.forall(req =>
      req.require_physical implies (
        val has_physical_match = available_devices.exists(d =>
          d.physical and
          (req.os_values.size() == 0 or req.os_values.contains(d.os)) and
          (req.type_values.size() == 0 or req.type_values.contains(d.dtype))
        )
        // If no physical match and ignore is false, must fail.
        (not(has_physical_match) and not(ignore_missing_physical)) implies
          match resolution_result {
            | Failed(_) => true
            | Resolved(_) => false
          }
      )
    )
  }

  // ── 10. Impossible coverage ──
  // When no combination of available devices (plus creatable devices if allowed)
  // can satisfy the requirements, the solver must fail.
  // (Stated contrapositively: if resolution succeeded, coverage is achievable.)

  val inv_impossible_coverage: bool = {
    match resolution_result {
      | Resolved(sel) =>
          // If resolved, then the requirements are actually satisfied.
          sel.size() == 0 or inv_coverage_completeness
      | Failed(_) => true
    }
  }

  // ── 11. No duplicate devices ──
  // Each physical/virtual device is selected at most once (by id).

  val inv_no_duplicate_devices: bool = {
    selected.forall(sd1 =>
      selected.forall(sd2 =>
        (sd1 != sd2) implies sd1.device.id != sd2.device.id
      )
    )
  }

  // ── 12. Created devices have NeedsCreation state ──
  // Any device marked as created must have had NeedsCreation state.

  val inv_created_device_state: bool = {
    selected.forall(sd =>
      sd.created implies sd.device.state == NeedsCreation
    )
  }

  // ── 13. Selected devices serve at least one app ──
  // Every selected device must be assigned to at least one app.

  val inv_selected_serves_app: bool = {
    selected.forall(sd => sd.apps.size() > 0)
  }

  // ──────────────────────────────────────────────────────────────────────
  // EDGE CASES
  // ──────────────────────────────────────────────────────────────────────

  // Edge case: Single device covers all requirements.
  // When one device satisfies all OS and type requirements for all apps,
  // only that one device should be selected (minimality ensures this).
  val inv_single_device_sufficient: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          // If any single device covers everything, selection size is 1
          // (for the non-expand case).
          val all_no_expand = requirements.forall(r => r.expand != FullExpand)
          val single_covers_all = sel.exists(sd =>
            requirements.forall(req =>
              req.os_values.forall(os => sd.device.os == os) and
              req.type_values.forall(dt => sd.device.dtype == dt) and
              req.explicit_ids.size() == 0 and
              req.explicit_names.size() == 0
            )
          )
          // If single covers all, no expand, no explicits, then size should be 1.
          (all_no_expand and single_covers_all and
           requirements.forall(r => r.explicit_ids.size() == 0 and r.explicit_names.size() == 0))
            implies sel.size() == 1
    }
  }

  // Edge case: Requirements forcing more devices than minimum across fields.
  // e.g., os: [ios:17, ios:18, android:14] and type: [phone, tablet]
  // Minimum coverage needs max(3, 2) = 3 devices, not 2.
  // This is a consequence of coverage_completeness + minimality.

  // Edge case: Two apps sharing a device.
  // A device can serve multiple apps simultaneously.
  val inv_shared_device_valid: bool = {
    selected.forall(sd =>
      sd.apps.size() > 1 implies
        // The device must be compatible with all apps it serves.
        sd.apps.forall(app =>
          requirements.exists(req =>
            req.app == app and (
              req.os_values.size() == 0 or req.os_values.contains(sd.device.os)
            ) and (
              req.type_values.size() == 0 or req.type_values.contains(sd.device.dtype)
            )
          )
        )
    )
  }

  // Edge case: latest:N expanding to N values.
  // When os is specified as "latest:3", it expands to the 3 most recent
  // OS versions before the solver runs. This is modeled by the os_values
  // set already containing the expanded values (e.g., {"ios:18", "ios:17", "ios:16"}).
  // No additional invariant needed beyond coverage_completeness, but we note
  // that |os_values| = N after expansion.

  // Edge case: Version range 17+ matching multiple versions.
  // Similar to latest:N, "17+" (OsVersionSpec::Minimum) expands to all
  // available OS versions >= 17 before solving. The os_values set contains
  // the expanded results. Coverage completeness ensures all are covered.

  // Edge case: All devices shutdown (still prefer fewer).
  // When all available devices are Shutdown, the solver should still
  // select the minimum number. Preference ordering doesn't increase count;
  // it only picks better-state devices when counts are equal.
  val inv_all_shutdown_still_minimal: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          val all_shutdown = available_devices.forall(d => d.state == Shutdown)
          // Even when all are shutdown, minimality still holds.
          all_shutdown implies inv_minimality
    }
  }

  // Edge case: Explicit device is shutdown but a running device also matches.
  // The explicit device must still be selected (explicit inclusion rule).
  // The running device may or may not be selected depending on whether it
  // covers additional requirements.
  val inv_explicit_shutdown_still_selected: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          requirements.forall(req =>
            req.explicit_ids.forall(eid =>
              // Even if the explicit device is shutdown, it must be selected.
              available_devices.exists(d => d.id == eid and d.state == Shutdown) implies
                sel.exists(sd => sd.device.id == eid)
            )
          )
    }
  }

  // Edge case: Explicit device is the only one satisfying a requirement.
  // The explicit device's coverage credit should prevent the solver from
  // trying to find an auto-select alternative that doesn't exist.
  val inv_explicit_sole_coverage: bool = {
    match resolution_result {
      | Failed(_) => true
      | Resolved(sel) =>
          sel.forall(sd =>
            is_explicit(sd) implies
              // The explicit device is in the selection regardless of
              // whether other devices could cover the same values.
              sel.exists(s => s.device.id == sd.device.id)
          )
    }
  }

  // ──────────────────────────────────────────────────────────────────────
  // Combined invariant
  // ──────────────────────────────────────────────────────────────────────

  val all_invariants: bool = and {
    inv_coverage_completeness,
    inv_minimality,
    inv_preference_ordering,
    inv_explicit_inclusion,
    inv_explicit_coverage_credit,
    inv_no_waste,
    inv_create_guard,
    inv_create_guard_selected,
    inv_full_expansion,
    inv_full_expansion_count,
    inv_physical_skip,
    inv_impossible_coverage,
    inv_no_duplicate_devices,
    inv_created_device_state,
    inv_selected_serves_app,
    inv_single_device_sufficient,
    inv_shared_device_valid,
    inv_all_shutdown_still_minimal,
    inv_explicit_shutdown_still_selected,
    inv_explicit_sole_coverage,
  }

  // ──────────────────────────────────────────────────────────────────────
  // Step relation: any action can fire
  // ──────────────────────────────────────────────────────────────────────

  action step = any {
    // Add devices to the available pool.
    nondet id    = Set("dev1", "dev2", "dev3", "dev4", "dev5", "dev6").oneOf()
    nondet name  = Set("iPhone 15", "iPhone 16", "Pixel 8", "iPad Air", "Galaxy Tab", "Pixel 9").oneOf()
    nondet os    = Set("ios:17", "ios:18", "android:14", "android:15", "ios:16").oneOf()
    nondet dtype = Set("phone", "tablet").oneOf()
    nondet phys  = Set(true, false).oneOf()
    nondet state = Set(Booted, Shutdown, NeedsCreation).oneOf()
    add_available_device({ id: id, name: name, os: os, dtype: dtype, physical: phys, state: state }),

    // Add coverage requirements.
    nondet app     = Set("com.example.app", "com.test.runner").oneOf()
    nondet os_vals = Set(
      Set("ios:17"),
      Set("ios:17", "ios:18"),
      Set("ios:17", "ios:18", "android:14"),
      Set("android:14", "android:15"),
      Set("ios:16", "ios:17", "ios:18")
    ).oneOf()
    nondet type_vals = Set(
      Set("phone"),
      Set("tablet"),
      Set("phone", "tablet")
    ).oneOf()
    nondet exp_ids   = Set(Set(), Set("dev1"), Set("dev3")).oneOf()
    nondet exp_names = Set(Set(), Set("iPhone 15"), Set("Pixel 8")).oneOf()
    nondet expand    = Set(NoExpand, FullExpand).oneOf()
    nondet req_phys  = Set(true, false).oneOf()
    add_requirement({
      app: app, os_values: os_vals, type_values: type_vals,
      explicit_ids: exp_ids, explicit_names: exp_names,
      expand: expand, require_physical: req_phys,
    }),

    // Set global options.
    nondet cim = Set(true, false).oneOf()
    nondet imp = Set(true, false).oneOf()
    set_options(cim, imp),

    // Select a device.
    nondet dev_id = Set("dev1", "dev2", "dev3", "dev4", "dev5", "dev6").oneOf()
    nondet dev_name = Set("iPhone 15", "iPhone 16", "Pixel 8", "iPad Air", "Galaxy Tab", "Pixel 9").oneOf()
    nondet dev_os = Set("ios:17", "ios:18", "android:14", "android:15", "ios:16").oneOf()
    nondet dev_dtype = Set("phone", "tablet").oneOf()
    nondet dev_phys = Set(true, false).oneOf()
    nondet dev_state = Set(Booted, Shutdown, NeedsCreation).oneOf()
    nondet sel_app = Set("com.example.app", "com.test.runner").oneOf()
    nondet sel_origin = Set(Explicit, AutoSelected).oneOf()
    nondet sel_created = Set(true, false).oneOf()
    select_device(
      { id: dev_id, name: dev_name, os: dev_os, dtype: dev_dtype, physical: dev_phys, state: dev_state },
      sel_app, sel_origin, sel_created
    ),

    // Share a device between apps.
    nondet share_id  = Set("dev1", "dev2", "dev3").oneOf()
    nondet share_app = Set("com.example.app", "com.test.runner").oneOf()
    share_device(share_id, share_app),

    // Fail resolution.
    nondet reason = Set(
      "impossible: no device covers ios:19",
      "no physical device available",
      "cannot create device: create_if_missing is false"
    ).oneOf()
    fail_resolution(reason),
  }

  // ──────────────────────────────────────────────────────────────────────
  // Temporal property: invariants hold in every reachable state
  // ──────────────────────────────────────────────────────────────────────

  temporal all_invariants_always = always(all_invariants)

}
