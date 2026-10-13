# shep-handover

Replace a process's image with `execve` and carry on: named descriptors and a
state blob cross, and the children the old image started stay the new image's
children. Any process can use it. It depends on no other shep crate.

```rust,no_run
use serde::{Deserialize, Serialize};
use shep_handover::{Handover, State, Verdict};

#[derive(Serialize, Deserialize)]
struct Carry {
    child: u32,
}

impl State for Carry {
    const FORMAT: u32 = 1;
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // First thing in main: is this image a successor?
    if let Some(mut arrival) = shep_handover::boot::<Carry>()? {
        let output = arrival.fds.take("child-out").expect("carried").into_reader()?;
        let exit = shep_handover::wait_for_child(arrival.state.child).await?;
        let _ = (output, exit);
        return Ok(());
    }

    let child_out = std::io::pipe()?.0;
    let state = Carry { child: 4242 };
    // Returns only if the handover did not happen; this image is still running.
    let refusal = Handover::new(&state)
        .carry("child-out", &child_out)
        .exec(&|_: &std::path::Path| Verdict::Carryable)
        .unwrap_err();
    eprintln!("staying on this image: {refusal}");
    Ok(())
}
```

- `Handover::carry` names each descriptor to carry. Only those have
  close-on-exec cleared; every other descriptor is closed in the new image.
- `State` is the blob, under a format number. A build that cannot read the
  format refuses it from `boot` with a reason.
- `Fitness` is the caller's gate. A refusal, an unreadable target or a failed
  `execve` leaves the old image running and returns the reason, so the caller
  can fall back to a stop and a start.
- Unix only. On Windows the crate builds and every entry point answers "not
  supported".

shep is pre-release. Anything public here can change before 1.0.

## License

MIT OR Apache-2.0, at your option.
