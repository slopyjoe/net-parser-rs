use super::prelude::*;
use super::{InternetProtocolId, Layer3FlowInfo};

use self::nom::*;
use self::layer4::{
    Layer4,
    Layer4FlowInfo,
    tcp::*,
    udp::*};
use std;
use std::convert::TryFrom;

const ADDRESS_LENGTH: usize = 16;
const HEADER_LENGTH: usize = 4 * std::mem::size_of::<u16>();

pub struct IPv6 {
    dst_ip: std::net::IpAddr,
    src_ip: std::net::IpAddr,
    protocol: InternetProtocolId,
    payload: std::vec::Vec<u8>
}

fn to_ip_address(i: &[u8]) -> std::net::IpAddr {
    let ipv6 = std::net::Ipv6Addr::from(array_ref![i, 0, ADDRESS_LENGTH].clone());
    std::net::IpAddr::V6(ipv6)
}

named!(
    ipv6_address<&[u8], std::net::IpAddr>,
    map!(take!(ADDRESS_LENGTH), to_ip_address)
);

impl IPv6 {
    pub fn dst_ip(&self) -> &std::net::IpAddr {
        &self.dst_ip
    }
    pub fn src_ip(&self) -> &std::net::IpAddr {
        &self.src_ip
    }
    pub fn protocol(&self) -> &InternetProtocolId {
        &self.protocol
    }
    pub fn payload(&self) -> &std::vec::Vec<u8> { &self.payload }

    fn parse_next_header(
        input: &[u8],
        payload_length: u16,
        next_header: InternetProtocolId
    ) -> IResult<&[u8], IPv6> {
        if InternetProtocolId::has_next_option(next_header.clone()) {
            let (rem, h) = do_parse!(input,

                h: map_opt!(be_u8, InternetProtocolId::new) >>

                ( h )
            )?;

            IPv6::parse_next_header(rem, payload_length, h)
        } else {
            do_parse!(input,

                _h: take!(1) >> //hop limit
                src: ipv6_address >>
                dst: ipv6_address >>
                payload: take!(payload_length) >>

                (
                    IPv6 {
                        dst_ip: dst,
                        src_ip: src,
                        protocol: next_header,
                        payload: payload.into()
                    }
                )
            )
        }
    }

    fn parse_ipv6(input: &[u8]) -> IResult<&[u8], IPv6> {
        let (rem, (payload_length, next_header)) = do_parse!(input,

            _f: take!(3) >> //version and flow label
            p: be_u16 >>
            h: map_opt!(be_u8, InternetProtocolId::new) >>

            ( (p, h) )
        )?;

        trace!("Payload Lengt={}", payload_length);

        IPv6::parse_next_header(rem, payload_length, next_header)
    }

    pub fn new(
        dst_ip: std::net::Ipv6Addr,
        src_ip: std::net::Ipv6Addr,
        protocol: InternetProtocolId,
        payload: std::vec::Vec<u8>
    ) -> IPv6 {
        IPv6 {
            dst_ip: std::net::IpAddr::V6(dst_ip),
            src_ip: std::net::IpAddr::V6(src_ip),
            protocol: protocol,
            payload: payload
        }
    }

    pub fn parse(input: &[u8]) -> IResult<&[u8], IPv6> {
        trace!("Available={}", input.len());

        be_u8(input).and_then(|r| {
            let (rem, length_check) = r;
            let version = length_check >> 4;
            if version == 6 {
                IPv6::parse_ipv6(rem)
            } else {
                Err(Err::convert(Err::Error(error_position!(input, ErrorKind::CondReduce::<u32>))))
            }
        })
    }
}

impl TryFrom<IPv6> for Layer3FlowInfo {
    type Error = errors::Error;

    fn try_from(value: IPv6) -> Result<Self, Self::Error> {
        debug!("Creating flow info from {:?}", value.protocol);
        let l4 = match value.protocol.clone() {
            InternetProtocolId::Tcp => {
                layer4::tcp::Tcp::parse(value.payload())
                    .map_err(|e| {
                        let err: Self::Error = e.into();
                        err.chain_err(|| errors::Error::from_kind(errors::ErrorKind::FlowParse))
                    }).and_then(|r| {
                    let (rem, l4) = r;
                    if rem.is_empty() {
                        Layer4FlowInfo::try_from(l4)
                    } else {
                        Err(errors::Error::from_kind(errors::ErrorKind::IncompleteParse(rem.len())))
                    }
                })
            }
            InternetProtocolId::Udp => {
                layer4::udp::Udp::parse(value.payload())
                    .map_err(|e| {
                        let err: Self::Error = e.into();
                        err.chain_err(|| errors::Error::from_kind(errors::ErrorKind::FlowParse))
                    }).and_then(|r| {
                    let (rem, l4) = r;
                    if rem.is_empty() {
                        Layer4FlowInfo::try_from(l4)
                    } else {
                        Err(errors::Error::from_kind(errors::ErrorKind::IncompleteParse(rem.len())))
                    }
                })
            }
            _ => {
                Err(errors::Error::from_kind(errors::ErrorKind::IPv4Type(value.protocol)))
            }
        }?;

        Ok(Layer3FlowInfo {
            src_ip: value.src_ip,
            dst_ip: value.dst_ip,
            layer4: l4
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate env_logger;
    extern crate hex_slice;
    use self::hex_slice::AsHex;

    use super::*;

    const RAW_DATA: &'static [u8] = &[
        0x65u8, //version and header length
        0x00u8, 0x00u8, 0x00u8, //traffic class and label
        0x00u8, 0x34u8, //payload length
        0x06u8, //next hop, protocol, tcp
        0x00u8, //hop limit
        0x01u8, 0x02u8, 0x03u8, 0x04u8, 0x05u8, 0x06u8, 0x07u8, 0x08u8, 0x09u8, 0x0Au8, 0x0Bu8, 0x0Cu8, 0x0Du8, 0x0Eu8, 0x0Fu8, 0x0Fu8,//src ip 12:34:56:78:9A:BC:DE:FF
        0x0Fu8, 0x00u8, 0x01u8, 0x02u8, 0x03u8, 0x04u8, 0x05u8, 0x06u8, 0x07u8, 0x08u8, 0x09u8, 0x0Au8, 0x0Bu8, 0x0Cu8, 0x0Du8, 0x0Eu8,//dst ip F0:12:34:56:78:9A:BC:DE
        //tcp
        0xC6u8, 0xB7u8, //src port, 50871
        0x00u8, 0x50u8, //dst port, 80
        0x00u8, 0x00u8, 0x00u8, 0x01u8, //sequence number, 1
        0x00u8, 0x00u8, 0x00u8, 0x02u8, //acknowledgement number, 2
        0x50u8, 0x00u8, //header and flags, 0
        0x00u8, 0x00u8, //window
        0x00u8, 0x00u8, //check
        0x00u8, 0x00u8, //urgent
        //no options
        //payload
        0x01u8, 0x02u8, 0x03u8, 0x04u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0xfcu8, 0xfdu8, 0xfeu8, 0xffu8 //payload, 8 words
    ];

    #[test]
    fn parse_ipv6() {
        let _ = env_logger::try_init();

        let (rem, l3) = IPv6::parse(RAW_DATA).expect("Unable to parse");

        assert_eq!(*l3.src_ip(), "102:304:506:708:90A:B0C:D0E:F0F".parse::<std::net::IpAddr>().expect("Could not parse ip address"));
        assert_eq!(*l3.dst_ip(), "F00:102:304:506:708:90A:B0C:D0E".parse::<std::net::IpAddr>().expect("Could not parse ip address"));

        let is_tcp = if let InternetProtocolId::Tcp = l3.protocol() {
            true
        } else {
            false
        };

        assert!(is_tcp);

        assert!(rem.is_empty());
    }
    #[test]
    fn convert_ipv6() {
        let _ = env_logger::try_init();

        let (rem, l3) = IPv6::parse(RAW_DATA).expect("Unable to parse");

        let info = Layer3FlowInfo::try_from(l3).expect("Could not convert to layer 3 info");

        assert_eq!(info.src_ip, "102:304:506:708:90A:B0C:D0E:F0F".parse::<std::net::IpAddr>().expect("Could not parse ip address"));
        assert_eq!(info.dst_ip, "F00:102:304:506:708:90A:B0C:D0E".parse::<std::net::IpAddr>().expect("Could not parse ip address"));
        assert_eq!(info.layer4.src_port, 50871);
        assert_eq!(info.layer4.dst_port, 80);
    }
}