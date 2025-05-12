use asrt::{Runtime, yield_now, async_callback};
use std::time::Instant;

#[test]
fn test1() {
    let rt = Runtime::new();
    println!("{:?}", std::thread::current().name());
    rt.run(async {
        println!("{:?}", std::thread::current().name());
        yield_now().await;
        println!("{:?}", std::thread::current().name());
    })
    .wait()
    .unwrap();
}

#[test]
fn test2() {
    let rt = Runtime::new();
    println!("{:?}", std::thread::current().name());
    let task0 = rt.run(async {
        println!("0 {:?}", std::thread::current().name());
        yield_now().await;
        println!("0 {:?}", std::thread::current().name());
    });
    rt.run(async {
        println!("1 {:?}", std::thread::current().name());
        println!("1 {:?}", task0.await);
        println!("1 {:?}", std::thread::current().name());
    })
    .wait()
    .unwrap();
}

#[test]
fn test3() {
    let rt = Runtime::new();
    let start = Instant::now();
    let tasks: Vec<_> = (0..100)
        .map(|i| {
            (
                i,
                rt.run(async move {
                    println!(
                        "A {i} {:?} {:?}",
                        std::thread::current().name(),
                        start.elapsed()
                    );
                    async_callback(|then| {
                        std::thread::spawn(move ||  {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            then.then(());
                        });
                    }).await;
                    println!(
                        "B {i} {:?} {:?}",
                        std::thread::current().name(),
                        start.elapsed()
                    );
                }),
            )
        })
        .collect();
    rt.run(async move {
        for (i, task) in tasks {
            println!(
                "Z {i} {:?} {:?} {:?}",
                task.await,
                std::thread::current().name(),
                start.elapsed()
            );
        }
    })
    .wait()
    .unwrap();
}
