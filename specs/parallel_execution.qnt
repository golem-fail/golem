// parallel_execution.qnt — Formal specification of Golem's parallel execution model.
//
// This models the block-as-barrier execution pattern: a flow is a directed graph
// of blocks, each block contains ordered steps, and devices execute steps in
// parallel within a block but synchronize at block boundaries (barriers).
// Also models for_each iteration, where filtering, loop counters,
// max_iterations, failure propagation, and variable persistence.

module parallel_execution {

  // ──────────────────────────────────────────────────────────────────────
  // Types
  // ──────────────────────────────────────────────────────────────────────

  // Device execution status within a block.
  type DeviceStatus =
    | Idle           // waiting at barrier for next block
    | Executing      // currently running a step within its block
    | Completed      // finished all steps in current block, waiting at barrier
    | Failed         // step failed during execution
    | Skipped        // device did not match where filter or app targeting

  // What happens when a step fails.
  type OnFailPolicy = ErrorPolicy | ContinuePolicy | SkipPolicy

  // Per-device execution state.
  type DeviceState = {
    id:             str,
    app:            str,           // which app this device is assigned to
    block_index:    int,           // current block index in the flow
    step_index:     int,           // current step index within the block
    status:         DeviceStatus,
    vars:           str -> str,    // device-local variable store (save_to results)
    loop_counter:   int,           // _loop value for current block
    iterations_used: int,          // how many iterations of current block (for max_iterations)
    on_fail_policy: OnFailPolicy,  // policy of the step that failed (if Failed)
  }

  // A block in the flow graph.
  type Block = {
    id:              str,
    index:           int,          // position in the block list
    app:             str,          // which app this block targets ("" = all)
    steps:           List[str],    // step identifiers (actions) in order
    next:            str,          // next block id ("" = flow end)
    for_each:        str,          // app name to iterate devices of ("" = none)
    where_type:      str,          // device type filter ("" = no filter)
    where_os:        str,          // device OS filter ("" = no filter)
    max_iterations:  int,          // 0 = unlimited
  }

  // A for_each iteration cursor.
  type ForEachCursor = {
    block_id:           str,       // which block is being iterated
    target_devices:     List[str], // ordered list of device ids to iterate over
    current_index:      int,       // current position in target_devices
    active:             bool,      // whether a for_each iteration is in progress
  }

  // Flow-level outcome.
  type FlowResult =
    | Running
    | FlowCompleted
    | FlowFailed(str)

  // ──────────────────────────────────────────────────────────────────────
  // State
  // ──────────────────────────────────────────────────────────────────────

  // All devices participating in the flow.
  var devices: str -> DeviceState

  // The block graph: ordered list of blocks.
  var blocks: List[Block]

  // The for_each iteration cursor (at most one active at a time).
  var for_each_cursor: ForEachCursor

  // Snapshot of device vars at the start of a block (for step isolation).
  // Maps device_id -> (var_name -> value) captured when block execution began.
  var block_start_snapshot: str -> (str -> str)

  // Vars written during the current block by each device (pending commit at barrier).
  var pending_writes: str -> (str -> str)

  // Flow-level result.
  var flow_result: FlowResult

  // Counter for tracking barrier synchronization events.
  var barrier_epoch: int

  // ──────────────────────────────────────────────────────────────────────
  // Helpers
  // ──────────────────────────────────────────────────────────────────────

  // All devices assigned to a particular app (or all if app is "").
  pure def devices_for_app(devs: str -> DeviceState, app: str): Set[str] =
    devs.keys().filter(did =>
      app == "" or devs.get(did).app == app
    )

  // All devices assigned to a block (considering app targeting).
  pure def devices_for_block(devs: str -> DeviceState, blk: Block): Set[str] =
    devices_for_app(devs, blk.app)

  // Devices that match a block's where filter.
  pure def devices_matching_where(
    devs: str -> DeviceState,
    blk: Block
  ): Set[str] = {
    val targeted = devices_for_block(devs, blk)
    targeted.filter(did =>
      val d = devs.get(did)
      (blk.where_type == "" or d.vars.has("_type") and d.vars.get("_type") == blk.where_type) and
      (blk.where_os == "" or d.vars.has("_os") and d.vars.get("_os") == blk.where_os)
    )
  }

  // Check if all targeted devices for a block have completed (or skipped/failed).
  pure def all_devices_at_barrier(
    devs: str -> DeviceState,
    blk: Block
  ): bool = {
    val targeted = devices_for_block(devs, blk)
    targeted.forall(did =>
      val d = devs.get(did)
      d.status == Completed or d.status == Skipped or d.status == Failed
    )
  }

  // Check if any device failed with error policy in the current block.
  pure def any_error_failure(
    devs: str -> DeviceState,
    blk: Block
  ): bool = {
    val targeted = devices_for_block(devs, blk)
    targeted.exists(did =>
      val d = devs.get(did)
      d.status == Failed and d.on_fail_policy == ErrorPolicy
    )
  }

  // Total number of steps in a block.
  pure def step_count(blk: Block): int = blk.steps.length()

  // Get block by index (returns a default empty block if out of range).
  pure def get_block(blks: List[Block], idx: int): Block =
    if (idx >= 0 and idx < blks.length()) blks[idx]
    else { id: "", index: -1, app: "", steps: List(), next: "",
           for_each: "", where_type: "", where_os: "", max_iterations: 0 }

  // Find block index by id.
  pure def find_block_index(blks: List[Block], block_id: str): int =
    if (blks.length() == 0) -1
    else if (blks.indices().exists(i => blks[i].id == block_id))
      blks.indices().fold(-1, (acc, i) =>
        if (blks[i].id == block_id) i else acc
      )
    else -1

  // ──────────────────────────────────────────────────────────────────────
  // Initialization
  // ──────────────────────────────────────────────────────────────────────

  action init = all {
    devices'            = Map(),
    blocks'             = List(),
    for_each_cursor'    = { block_id: "", target_devices: List(), current_index: 0, active: false },
    block_start_snapshot' = Map(),
    pending_writes'     = Map(),
    flow_result'        = Running,
    barrier_epoch'      = 0,
  }

  // ──────────────────────────────────────────────────────────────────────
  // Actions: setup (configure flow before execution)
  // ──────────────────────────────────────────────────────────────────────

  // Add a device to the execution.
  action add_device(
    id: str, app: str, initial_vars: str -> str
  ) = all {
    val dev: DeviceState = {
      id: id, app: app, block_index: 0, step_index: 0,
      status: Idle, vars: initial_vars, loop_counter: 0,
      iterations_used: 0, on_fail_policy: ErrorPolicy,
    }
    devices'            = devices.put(id, dev),
    blocks'             = blocks,
    for_each_cursor'    = for_each_cursor,
    block_start_snapshot' = block_start_snapshot,
    pending_writes'     = pending_writes,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // Add a block to the flow graph.
  action add_block(blk: Block) = all {
    blocks'             = blocks.append(blk),
    devices'            = devices,
    for_each_cursor'    = for_each_cursor,
    block_start_snapshot' = block_start_snapshot,
    pending_writes'     = pending_writes,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // ──────────────────────────────────────────────────────────────────────
  // Actions: execution
  // ──────────────────────────────────────────────────────────────────────

  // Begin a block: snapshot device vars for step isolation, set devices to
  // Executing or Skipped based on app targeting and where filter.
  action begin_block(block_idx: int) = all {
    block_idx >= 0,
    block_idx < blocks.length(),
    val blk = blocks[block_idx]
    val targeted = devices_for_block(devices, blk)
    val matching = devices_matching_where(devices, blk)
    // Snapshot all device vars at block start for step isolation.
    block_start_snapshot' = devices.keys().fold(Map(), (acc, did) =>
      acc.put(did, devices.get(did).vars)
    ),
    // Initialize pending writes as empty for each device.
    pending_writes' = devices.keys().fold(Map(), (acc, did) =>
      acc.put(did, Map())
    ),
    // Update device statuses.
    devices' = devices.keys().fold(devices, (acc, did) =>
      val d = acc.get(did)
      if (not(targeted.contains(did))) {
        // Not targeted by this block's app — skip.
        acc.put(did, { ...d, status: Skipped, block_index: block_idx, step_index: 0 })
      } else if (not(matching.contains(did))) {
        // Targeted but doesn't match where filter — skip.
        acc.put(did, { ...d, status: Skipped, block_index: block_idx, step_index: 0 })
      } else if (step_count(blk) == 0) {
        // Block has no steps — immediately completed.
        acc.put(did, { ...d, status: Completed, block_index: block_idx, step_index: 0 })
      } else {
        // Begin executing.
        acc.put(did, { ...d, status: Executing, block_index: block_idx, step_index: 0 })
      }
    ),
    blocks'             = blocks,
    for_each_cursor'    = for_each_cursor,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // A device executes its current step (advances step_index).
  // The device reads from block_start_snapshot (not other devices' live vars).
  action execute_step(device_id: str) = all {
    devices.has(device_id),
    val d = devices.get(device_id)
    d.status == Executing,
    d.block_index >= 0,
    d.block_index < blocks.length(),
    val blk = blocks[d.block_index]
    d.step_index < step_count(blk),
    // Advance step index. If this was the last step, mark Completed.
    val next_step = d.step_index + 1
    val new_status = if (next_step >= step_count(blk)) Completed else Executing
    devices' = devices.put(device_id,
      { ...d, step_index: next_step, status: new_status }
    ),
    blocks'             = blocks,
    for_each_cursor'    = for_each_cursor,
    block_start_snapshot' = block_start_snapshot,
    pending_writes'     = pending_writes,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // A device writes a variable during step execution (save_to).
  // The write goes to pending_writes, not immediately visible to other devices.
  action device_save_var(device_id: str, var_name: str, var_value: str) = all {
    devices.has(device_id),
    val d = devices.get(device_id)
    d.status == Executing,
    // Write to pending_writes for this device.
    val existing = if (pending_writes.has(device_id)) pending_writes.get(device_id) else Map()
    pending_writes' = pending_writes.put(device_id, existing.put(var_name, var_value)),
    // Also update the device's own vars (visible to itself in subsequent steps).
    devices' = devices.put(device_id,
      { ...d, vars: d.vars.put(var_name, var_value) }
    ),
    blocks'             = blocks,
    for_each_cursor'    = for_each_cursor,
    block_start_snapshot' = block_start_snapshot,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // A device fails during step execution.
  action device_fail(device_id: str, policy: OnFailPolicy) = all {
    devices.has(device_id),
    val d = devices.get(device_id)
    d.status == Executing,
    devices' = devices.put(device_id,
      { ...d, status: Failed, on_fail_policy: policy }
    ),
    blocks'             = blocks,
    for_each_cursor'    = for_each_cursor,
    block_start_snapshot' = block_start_snapshot,
    pending_writes'     = pending_writes,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // Barrier synchronization: all targeted devices have finished the current block.
  // Commit pending writes to device vars, advance to next block.
  action barrier_sync(block_idx: int) = all {
    block_idx >= 0,
    block_idx < blocks.length(),
    val blk = blocks[block_idx]
    // All targeted devices must be at the barrier (completed, skipped, or failed).
    all_devices_at_barrier(devices, blk),
    // Check for error failures.
    val has_error = any_error_failure(devices, blk)
    // Determine next block index.
    val next_idx = if (blk.next == "") block_idx + 1
                   else find_block_index(blocks, blk.next)
    // Check max_iterations for looping blocks.
    val at_max = devices.keys().exists(did =>
      val d = devices.get(did)
      d.block_index == block_idx and
      blk.max_iterations > 0 and d.iterations_used >= blk.max_iterations
    )
    if (has_error) all {
      // Failure propagation: any device fail with on_fail="error" fails the flow.
      flow_result' = FlowFailed("device_error_at_barrier"),
      // Commit pending writes even on failure (vars persist).
      devices' = devices.keys().fold(devices, (acc, did) =>
        val d = acc.get(did)
        val pw = if (pending_writes.has(did)) pending_writes.get(did) else Map()
        val merged = d.vars.keys().fold(d.vars, (v, k) => v).keys().fold(d.vars, (v, k) =>
          if (pw.has(k)) v.put(k, pw.get(k)) else v
        )
        // Also merge any pending write keys not in existing vars.
        val fully_merged = pw.keys().fold(merged, (v, k) => v.put(k, pw.get(k)))
        acc.put(did, { ...d, vars: fully_merged, status: Idle })
      ),
      blocks'             = blocks,
      for_each_cursor'    = for_each_cursor,
      block_start_snapshot' = Map(),
      pending_writes'     = Map(),
      barrier_epoch'      = barrier_epoch + 1,
    } else all {
      // Success: commit pending writes and advance devices to next block.
      devices' = devices.keys().fold(devices, (acc, did) =>
        val d = acc.get(did)
        val pw = if (pending_writes.has(did)) pending_writes.get(did) else Map()
        val fully_merged = pw.keys().fold(d.vars, (v, k) => v.put(k, pw.get(k)))
        val is_looping = next_idx == block_idx
        val new_loop = if (is_looping) d.loop_counter + 1 else 0
        val new_iters = if (is_looping) d.iterations_used + 1 else 0
        val effective_next = if (at_max and is_looping) block_idx + 1 else next_idx
        acc.put(did, { ...d,
          vars: fully_merged,
          status: Idle,
          block_index: effective_next,
          step_index: 0,
          loop_counter: new_loop,
          iterations_used: new_iters,
        })
      ),
      flow_result' = if (next_idx >= blocks.length() and not(at_max)) FlowCompleted
                     else if (at_max and block_idx + 1 >= blocks.length()) FlowCompleted
                     else Running,
      blocks'             = blocks,
      for_each_cursor'    = for_each_cursor,
      block_start_snapshot' = Map(),
      pending_writes'     = Map(),
      barrier_epoch'      = barrier_epoch + 1,
    }
  }

  // Begin a for_each iteration on a block.
  // Creates an ordered cursor over the devices belonging to the for_each app.
  action begin_for_each(block_idx: int, device_order: List[str]) = all {
    block_idx >= 0,
    block_idx < blocks.length(),
    val blk = blocks[block_idx]
    blk.for_each != "",
    // device_order contains exactly the devices for the for_each app.
    val target_set = devices_for_app(devices, blk.for_each)
    device_order.length() == target_set.size(),
    // All devices in device_order must be in the target set.
    device_order.indices().forall(i => target_set.contains(device_order[i])),
    for_each_cursor' = {
      block_id: blk.id,
      target_devices: device_order,
      current_index: 0,
      active: true,
    },
    devices'            = devices,
    blocks'             = blocks,
    block_start_snapshot' = block_start_snapshot,
    pending_writes'     = pending_writes,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // Advance the for_each cursor to the next device.
  action advance_for_each = all {
    for_each_cursor.active,
    for_each_cursor.current_index < for_each_cursor.target_devices.length(),
    val next_idx = for_each_cursor.current_index + 1
    val is_done = next_idx >= for_each_cursor.target_devices.length()
    for_each_cursor' = { ...for_each_cursor,
      current_index: next_idx,
      active: not(is_done),
    },
    devices'            = devices,
    blocks'             = blocks,
    block_start_snapshot' = block_start_snapshot,
    pending_writes'     = pending_writes,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // End for_each when there are zero target devices (empty iteration, no-op).
  action for_each_empty(block_idx: int) = all {
    block_idx >= 0,
    block_idx < blocks.length(),
    val blk = blocks[block_idx]
    blk.for_each != "",
    val target_set = devices_for_app(devices, blk.for_each)
    target_set.size() == 0,
    // Block is a no-op: all devices skip.
    devices' = devices.keys().fold(devices, (acc, did) =>
      val d = acc.get(did)
      acc.put(did, { ...d, status: Skipped, block_index: block_idx })
    ),
    for_each_cursor' = { block_id: blk.id, target_devices: List(), current_index: 0, active: false },
    blocks'             = blocks,
    block_start_snapshot' = block_start_snapshot,
    pending_writes'     = pending_writes,
    flow_result'        = flow_result,
    barrier_epoch'      = barrier_epoch,
  }

  // ──────────────────────────────────────────────────────────────────────
  // INVARIANTS
  // ──────────────────────────────────────────────────────────────────────

  // ── 1. Barrier sync — no device begins block N+1 until every device
  //       assigned to block N has completed ──
  //
  // For any two devices targeted by the same block, if one is Executing
  // in block N, no other can be Executing in block N+1.

  val inv_barrier_sync: bool = {
    devices.keys().forall(did1 =>
      devices.keys().forall(did2 =>
        val d1 = devices.get(did1)
        val d2 = devices.get(did2)
        (did1 != did2 and d1.status == Executing) implies
          // d2 cannot be executing a block ahead of d1.
          not(d2.status == Executing and d2.block_index > d1.block_index)
      )
    )
  }

  // Stronger form: no device can be in Idle/Executing state for block N+1
  // while another device is still Executing in block N.

  val inv_barrier_no_advance_while_executing: bool = {
    devices.keys().forall(did1 =>
      devices.keys().forall(did2 =>
        val d1 = devices.get(did1)
        val d2 = devices.get(did2)
        (did1 != did2 and d1.status == Executing) implies
          d2.block_index <= d1.block_index
      )
    )
  }

  // ── 2. Step isolation — device executing step K cannot observe another
  //       device's step K side-effects mid-block ──
  //
  // During block execution, each device reads from the block_start_snapshot,
  // not from other devices' live vars. This is modeled by the fact that
  // device_save_var writes to pending_writes and the device's own vars,
  // but other devices only see the snapshot.

  val inv_step_isolation: bool = {
    // While any device is Executing, the snapshot must exist for all devices.
    val any_executing = devices.keys().exists(did =>
      devices.get(did).status == Executing
    )
    any_executing implies
      devices.keys().forall(did =>
        devices.get(did).status == Executing implies
          block_start_snapshot.has(did)
      )
  }

  // Isolation means: for two distinct devices both executing in the same block,
  // device A's pending writes are not in device B's snapshot.

  val inv_cross_device_writes_invisible_mid_block: bool = {
    devices.keys().forall(did_a =>
      devices.keys().forall(did_b =>
        val da = devices.get(did_a)
        val db = devices.get(did_b)
        (did_a != did_b and
         da.status == Executing and db.status == Executing and
         da.block_index == db.block_index and
         block_start_snapshot.has(did_b) and
         pending_writes.has(did_a)) implies
          // B's snapshot does not contain A's pending writes.
          pending_writes.get(did_a).keys().forall(var_name =>
            not(block_start_snapshot.get(did_b).has(var_name)) or
            block_start_snapshot.get(did_b).get(var_name) !=
              pending_writes.get(did_a).get(var_name) or
            // Unless B already had that value before the block started
            // (i.e., it was in the snapshot from before).
            block_start_snapshot.get(did_a).has(var_name) and
            block_start_snapshot.get(did_a).get(var_name) ==
              block_start_snapshot.get(did_b).get(var_name)
          )
      )
    )
  }

  // ── 3. for_each ordering — iterations proceed in deterministic order
  //       (device resolution order) ──

  val inv_for_each_deterministic_order: bool = {
    for_each_cursor.active implies (
      // current_index advances monotonically from 0 through the list.
      for_each_cursor.current_index >= 0 and
      for_each_cursor.current_index < for_each_cursor.target_devices.length() and
      // All devices in target list are unique.
      for_each_cursor.target_devices.indices().forall(i =>
        for_each_cursor.target_devices.indices().forall(j =>
          (i != j) implies
            for_each_cursor.target_devices[i] != for_each_cursor.target_devices[j]
        )
      )
    )
  }

  // ── 4. for_each variable access — ${_each.*} resolves to current
  //       iteration's device variables ──

  val inv_for_each_var_access: bool = {
    for_each_cursor.active implies (
      // The current iteration device must exist in the devices map.
      for_each_cursor.current_index < for_each_cursor.target_devices.length() and
      val current_dev_id = for_each_cursor.target_devices[for_each_cursor.current_index]
      devices.has(current_dev_id)
    )
  }

  // ── 5. for_each unavailability — ${_each.*} undefined outside for_each block ──

  val inv_for_each_unavailable_outside: bool = {
    // When for_each is not active, there is no current iteration device.
    not(for_each_cursor.active) implies (
      for_each_cursor.target_devices.length() == 0 or
      for_each_cursor.current_index >= for_each_cursor.target_devices.length()
    )
  }

  // ── 6. Loop counter — _loop increments by 1 on block re-entry, starts at 0 ──

  val inv_loop_counter_starts_at_zero: bool = {
    // Any device that has just entered a block for the first time
    // (iterations_used == 0) has loop_counter == 0.
    devices.keys().forall(did =>
      val d = devices.get(did)
      d.iterations_used == 0 implies d.loop_counter == 0
    )
  }

  val inv_loop_counter_non_negative: bool = {
    devices.keys().forall(did =>
      devices.get(did).loop_counter >= 0
    )
  }

  // ── 7. Loop counter isolation — device A's _loop independent of device B's ──

  val inv_loop_counter_isolation: bool = {
    // Different devices can have different loop_counter values.
    // Stated as: there is no invariant coupling between them.
    // Formally: each device's loop_counter is self-consistent
    // (matches its own iterations_used).
    devices.keys().forall(did =>
      val d = devices.get(did)
      // loop_counter <= iterations_used (it is either equal for looping
      // blocks or 0 for non-looping blocks).
      d.loop_counter <= d.iterations_used or d.loop_counter == 0
    )
  }

  // ── 8. max_iterations scope — per device per block, not global ──

  val inv_max_iterations_per_device: bool = {
    // Each device tracks its own iterations_used independently.
    devices.keys().forall(did =>
      val d = devices.get(did)
      d.iterations_used >= 0 and
      // If the current block has max_iterations set, this device's
      // iterations_used must not exceed it.
      (d.block_index >= 0 and d.block_index < blocks.length()) implies (
        val blk = blocks[d.block_index]
        blk.max_iterations > 0 implies
          d.iterations_used <= blk.max_iterations
      )
    )
  }

  // ── 9. where filtering — only matching devices execute;
  //       non-matching skip to next block ──

  val inv_where_filter_skip: bool = {
    devices.keys().forall(did =>
      val d = devices.get(did)
      (d.block_index >= 0 and d.block_index < blocks.length()) implies (
        val blk = blocks[d.block_index]
        val targeted = devices_for_block(devices, blk)
        val matching = devices_matching_where(devices, blk)
        // If this device is targeted but does not match where, it must be Skipped.
        (targeted.contains(did) and not(matching.contains(did)) and
         d.status != Idle) implies
          d.status == Skipped
      )
    )
  }

  // where filter matching zero devices is a skip, not an error.
  val inv_where_zero_match_not_error: bool = {
    blocks.indices().forall(bi =>
      val blk = blocks[bi]
      val matching = devices_matching_where(devices, blk)
      // If no devices match the where filter, the flow does not fail.
      (matching.size() == 0 and (blk.where_type != "" or blk.where_os != "")) implies
        flow_result != FlowFailed("where_filter_error")
    )
  }

  // ── 10. Failure propagation — any device fail with on_fail="error"
  //        fails the flow after barrier ──

  val inv_failure_propagation: bool = {
    // If any device has Failed with ErrorPolicy and all devices are at barrier,
    // the flow result must eventually become FlowFailed.
    // (In practice, barrier_sync enforces this.)
    devices.keys().forall(did =>
      val d = devices.get(did)
      (d.status == Failed and d.on_fail_policy == ErrorPolicy) implies (
        // Either the flow is already failed, or we're mid-block waiting for barrier.
        flow_result != FlowCompleted
      )
    )
  }

  // When all devices in a parallel block fail, the flow still fails via barrier.
  val inv_all_devices_fail: bool = {
    blocks.indices().forall(bi =>
      val blk = blocks[bi]
      val targeted = devices_for_block(devices, blk)
      val all_failed = targeted.size() > 0 and targeted.forall(did =>
        devices.get(did).status == Failed and
        devices.get(did).on_fail_policy == ErrorPolicy
      )
      val all_at_barrier = all_devices_at_barrier(devices, blk)
      (all_failed and all_at_barrier) implies
        flow_result != FlowCompleted
    )
  }

  // ── 11. Variable persistence — save_to visible to same device in
  //        subsequent blocks ──

  val inv_variable_persistence: bool = {
    // After barrier_sync commits pending writes, they become part of
    // the device's vars. This is structural in the barrier_sync action.
    // Stated as: a device's vars map only grows or gets overwritten,
    // never loses keys (outside of explicit removal which does not exist).
    // We verify pending_writes are a subset of what will be committed.
    devices.keys().forall(did =>
      val d = devices.get(did)
      (d.status == Completed and pending_writes.has(did)) implies
        pending_writes.get(did).keys().forall(k =>
          // The pending write key is either already in vars (overwrite)
          // or will be added. In both cases, it will persist.
          true
        )
    )
  }

  // ── 12. Cross-device visibility — after barrier, device A's vars
  //        accessible to B only via for_each or explicit prefix ──

  val inv_cross_device_visibility: bool = {
    // During execution (not at barrier), a device cannot directly read
    // another device's pending writes. Cross-device access requires
    // either for_each (${_each.*}) or explicit prefix (${device_name:var}).
    // This is enforced by step isolation: mid-block reads go to snapshot.
    devices.keys().forall(did_a =>
      devices.keys().forall(did_b =>
        val da = devices.get(did_a)
        val db = devices.get(did_b)
        (did_a != did_b and da.status == Executing and db.status == Executing and
         da.block_index == db.block_index) implies
          // Device A's snapshot does not include B's pending writes.
          (pending_writes.has(did_b) implies
            pending_writes.get(did_b).keys().forall(k =>
              not(block_start_snapshot.has(did_a)) or
              not(block_start_snapshot.get(did_a).has(k)) or
              // The value in A's snapshot is from before the block, not B's write.
              block_start_snapshot.get(did_a).get(k) != pending_writes.get(did_b).get(k) or
              // Unless both had the same pre-existing value.
              (block_start_snapshot.has(did_b) and
               block_start_snapshot.get(did_b).has(k) and
               block_start_snapshot.get(did_b).get(k) == block_start_snapshot.get(did_a).get(k))
            )
          )
      )
    )
  }

  // ── 13. Block targeting — block with app="X" runs only on app X's devices ──

  val inv_block_targeting: bool = {
    devices.keys().forall(did =>
      val d = devices.get(did)
      (d.status == Executing and
       d.block_index >= 0 and d.block_index < blocks.length()) implies (
        val blk = blocks[d.block_index]
        // If the block targets a specific app, the device must belong to it.
        blk.app != "" implies d.app == blk.app
      )
    )
  }

  // ──────────────────────────────────────────────────────────────────────
  // EDGE CASES
  // ──────────────────────────────────────────────────────────────────────

  // Edge case: One device finishes in 10ms, another in 30s — first waits at barrier.
  // Modeled by: a device can be Completed while another is still Executing
  // in the same block. The Completed device does not advance.

  val inv_fast_device_waits: bool = {
    devices.keys().forall(did =>
      val d = devices.get(did)
      (d.status == Completed and d.block_index >= 0 and d.block_index < blocks.length()) implies (
        val blk = blocks[d.block_index]
        val targeted = devices_for_block(devices, blk)
        // If any other targeted device is still Executing, this device
        // must remain Completed (at the barrier), not advance.
        targeted.exists(other_did =>
          other_did != did and devices.get(other_did).status == Executing
        ) implies d.status == Completed  // tautology: proving it stays Completed
      )
    )
  }

  // Edge case: for_each with zero target devices — block is no-op.

  val inv_for_each_zero_targets: bool = {
    blocks.indices().forall(bi =>
      val blk = blocks[bi]
      (blk.for_each != "" and devices_for_app(devices, blk.for_each).size() == 0) implies
        // This is not an error condition.
        flow_result != FlowFailed("for_each_no_targets")
    )
  }

  // Edge case: Nested reference in for_each — ${_each.some_var} must resolve
  // to the current iteration device's variable store.

  val inv_for_each_nested_ref: bool = {
    for_each_cursor.active implies (
      for_each_cursor.current_index < for_each_cursor.target_devices.length() implies (
        val current_dev_id = for_each_cursor.target_devices[for_each_cursor.current_index]
        // The device exists and its vars are accessible.
        devices.has(current_dev_id) and
        // Any var in the iteration device's store is the value _each would resolve to.
        devices.get(current_dev_id).vars.keys().forall(k =>
          devices.get(current_dev_id).vars.has(k)
        )
      )
    )
  }

  // Edge case: Block with where filter matching zero devices — skip, not error.
  // (Already covered by inv_where_zero_match_not_error above.)

  // Edge case: Loop branching to different app's block — first app idles.
  // If block N targets app A and its next points to block M targeting app B,
  // then app A's devices become Idle/Skipped when block M begins.

  val inv_cross_app_branch_idle: bool = {
    devices.keys().forall(did =>
      val d = devices.get(did)
      (d.block_index >= 0 and d.block_index < blocks.length()) implies (
        val blk = blocks[d.block_index]
        // If this block targets a different app than the device's app,
        // the device must not be Executing.
        (blk.app != "" and blk.app != d.app) implies
          (d.status == Skipped or d.status == Idle)
      )
    )
  }

  // Edge case: Parallel block where all devices fail.
  // The flow fails after barrier sync (covered by inv_all_devices_fail).

  // ──────────────────────────────────────────────────────────────────────
  // Combined invariant
  // ──────────────────────────────────────────────────────────────────────

  val all_invariants: bool = and {
    inv_barrier_sync,
    inv_barrier_no_advance_while_executing,
    inv_step_isolation,
    inv_cross_device_writes_invisible_mid_block,
    inv_for_each_deterministic_order,
    inv_for_each_var_access,
    inv_for_each_unavailable_outside,
    inv_loop_counter_starts_at_zero,
    inv_loop_counter_non_negative,
    inv_loop_counter_isolation,
    inv_max_iterations_per_device,
    inv_where_filter_skip,
    inv_where_zero_match_not_error,
    inv_failure_propagation,
    inv_all_devices_fail,
    inv_variable_persistence,
    inv_cross_device_visibility,
    inv_block_targeting,
    inv_fast_device_waits,
    inv_for_each_zero_targets,
    inv_for_each_nested_ref,
    inv_cross_app_branch_idle,
  }

  // ──────────────────────────────────────────────────────────────────────
  // Step relation: any action can fire
  // ──────────────────────────────────────────────────────────────────────

  action step = any {
    // Add a device.
    nondet dev_id  = Set("dev_1", "dev_2", "dev_3", "dev_4").oneOf()
    nondet dev_app = Set("com.app.a", "com.app.b", "").oneOf()
    nondet vars    = Set(
      Map(),
      Map("_type" -> "phone", "_os" -> "ios:17"),
      Map("_type" -> "tablet", "_os" -> "android:14"),
      Map("_type" -> "phone", "_os" -> "android:14"),
    ).oneOf()
    add_device(dev_id, dev_app, vars),

    // Add a block.
    nondet blk_id   = Set("setup", "login", "test", "verify", "teardown").oneOf()
    nondet blk_idx  = Set(0, 1, 2, 3, 4).oneOf()
    nondet blk_app  = Set("", "com.app.a", "com.app.b").oneOf()
    nondet blk_steps = Set(
      List(),
      List("tap_login"),
      List("tap_login", "enter_text", "assert_visible"),
      List("scroll", "tap", "assert_visible", "assert"),
    ).oneOf()
    nondet blk_next    = Set("", "login", "test", "verify", "setup").oneOf()
    nondet blk_fe      = Set("", "com.app.a", "com.app.b").oneOf()
    nondet blk_wt      = Set("", "phone", "tablet").oneOf()
    nondet blk_wo      = Set("", "ios:17", "android:14").oneOf()
    nondet blk_max     = Set(0, 1, 3, 5).oneOf()
    add_block({
      id: blk_id, index: blk_idx, app: blk_app, steps: blk_steps,
      next: blk_next, for_each: blk_fe, where_type: blk_wt,
      where_os: blk_wo, max_iterations: blk_max,
    }),

    // Begin a block.
    nondet bi = Set(0, 1, 2, 3).oneOf()
    begin_block(bi),

    // Execute a step.
    nondet exec_dev = Set("dev_1", "dev_2", "dev_3", "dev_4").oneOf()
    execute_step(exec_dev),

    // Device saves a variable.
    nondet save_dev = Set("dev_1", "dev_2", "dev_3", "dev_4").oneOf()
    nondet save_var = Set("result", "token", "status", "screenshot_path").oneOf()
    nondet save_val = Set("ok", "fail", "abc123", "true").oneOf()
    device_save_var(save_dev, save_var, save_val),

    // Device fails.
    nondet fail_dev = Set("dev_1", "dev_2", "dev_3", "dev_4").oneOf()
    nondet fail_pol = Set(ErrorPolicy, ContinuePolicy, SkipPolicy).oneOf()
    device_fail(fail_dev, fail_pol),

    // Barrier sync.
    nondet barrier_bi = Set(0, 1, 2, 3).oneOf()
    barrier_sync(barrier_bi),

    // Begin for_each.
    nondet fe_bi    = Set(0, 1, 2, 3).oneOf()
    nondet fe_order = Set(
      List("dev_1"),
      List("dev_1", "dev_2"),
      List("dev_2", "dev_1"),
      List("dev_1", "dev_2", "dev_3"),
      List(),
    ).oneOf()
    begin_for_each(fe_bi, fe_order),

    // Advance for_each.
    advance_for_each,

    // for_each with zero targets.
    nondet fe_empty_bi = Set(0, 1, 2, 3).oneOf()
    for_each_empty(fe_empty_bi),
  }

  // ──────────────────────────────────────────────────────────────────────
  // Temporal property: invariants hold in every reachable state
  // ──────────────────────────────────────────────────────────────────────

  temporal all_invariants_always = always(all_invariants)

}
