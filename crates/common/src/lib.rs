mod models {
    pub mod spark {
        pub mod connect {
            include!("gen/spark/connect/spark.connect.rs");
        }
    }
    pub mod delta {
        pub mod connect {
            include!("gen/delta/connect/delta.connect.rs");
        }
    }
}

prost_message_ext!{}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
