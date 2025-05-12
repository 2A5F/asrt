use asrt::{Runtime, yield_now};

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
