use super::prelude::*;

use self::nom::*;
use self::layer3::{
    Layer3,
    Layer3FlowInfo,
    ipv4::*
};

use std;
use std::convert::TryFrom;
use super::Layer2FlowInfo;

const ETHERNET_PAYLOAD: u16 = 1500u16;
const VLAN_LENGTH: usize = 4;

///
/// List of valid ethernet types that aren't payload or vlan. https://en.wikipedia.org/wiki/EtherType
///
#[derive(Clone, Debug, PartialEq)]
pub enum Layer3Id {
    Lldp,
    IPv4,
    IPv6,
    Arp
}

#[derive(Clone, Debug, PartialEq)]
pub enum VlanTypeId {
    VlanTagId,
    ProviderBridging,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EthernetTypeId {
    PayloadLength(u16),
    Vlan(VlanTypeId),
    L3(Layer3Id)
}

impl EthernetTypeId {
    fn new(vlan: u16) -> Option<EthernetTypeId> {
        match vlan {
            0x8100u16 => Some(EthernetTypeId::Vlan(VlanTypeId::VlanTagId)),
            0x88a8u16 => Some(EthernetTypeId::Vlan(VlanTypeId::ProviderBridging)),
            0x88ccu16 => Some(EthernetTypeId::L3(Layer3Id::Lldp)),
            0x0800u16 => Some(EthernetTypeId::L3(Layer3Id::IPv4)),
            0x86ddu16 => Some(EthernetTypeId::L3(Layer3Id::IPv6)),
            0x0806u16 => Some(EthernetTypeId::L3(Layer3Id::Arp)),
            x if x <= ETHERNET_PAYLOAD => Some(EthernetTypeId::PayloadLength(x)),
            x => {
                //TODO: change to warn once list is more complete
                debug!("Encountered {:02x} when parsing Ethernet type", vlan);
                None
            }
        }
    }
}

pub struct VlanTag {
    vlan_type: VlanTypeId,
    value: [u8; 4]
}

impl VlanTag {
    pub fn vlan(&self) -> u16 {
        unsafe { std::mem::transmute::<[u8; 2], u16>(array_ref!(self.value, 2, 2).clone()) }
    }
}

pub struct Ethernet {
    dst_mac: MacAddress,
    src_mac: MacAddress,
    ether_type: EthernetTypeId,
    vlans: std::vec::Vec<VlanTag>,
    payload: std::vec::Vec<u8>
}

fn to_mac_address(i: &[u8]) -> MacAddress {
    MacAddress(array_ref![i, 0, MAC_LENGTH].clone())
}

named!(mac_address<&[u8], MacAddress>, map!(take!(MAC_LENGTH), to_mac_address));

impl Ethernet {
    pub fn dst_mac(&self) -> &MacAddress {
        &self.dst_mac
    }

    pub fn src_mac(&self) -> &MacAddress {
        &self.src_mac
    }

    pub fn ether_type(&self) -> &EthernetTypeId {
        &self.ether_type
    }

    pub fn vlans(&self) -> &std::vec::Vec<VlanTag> {
        &self.vlans
    }

    pub fn vlans_to_vlan(vlans: &std::vec::Vec<VlanTag>) -> Vlan {
        let opt_vlan = vlans.first().map(|v| v.vlan());
        opt_vlan.unwrap_or(0)
    }

    pub fn vlan(&self) -> Vlan {
        Ethernet::vlans_to_vlan(&self.vlans)
    }

    pub fn payload(&self) -> &std::vec::Vec<u8> {
        &self.payload
    }

    fn parse_with_existing_vlan_tag<'b>(
        input: &'b [u8],
        dst_mac: MacAddress,
        src_mac: MacAddress,
        vlan_type: VlanTypeId,
        agg: std::vec::Vec<VlanTag>
    ) -> nom::IResult<&'b [u8], Ethernet> {
        take!(input, VLAN_LENGTH).and_then(|r| {
            let (rem, vlan) = r;
            let mut agg_mut = agg;
            agg_mut.push(VlanTag {
                vlan_type: vlan_type,
                value: array_ref!(vlan, 0, VLAN_LENGTH).clone()
            });
            Ethernet::parse_vlan_tag(rem, dst_mac, src_mac, agg_mut)
        })
    }

    fn parse_vlan_tag(
        input: &[u8],
        dst_mac: MacAddress,
        src_mac: MacAddress,
        agg: std::vec::Vec<VlanTag>
    ) -> nom::IResult<&[u8], Ethernet> {
        let vlan_res = do_parse!(input,

            vlan: map_opt!(be_u16, EthernetTypeId::new) >>

            (vlan)
        );

        vlan_res.and_then(|r| {
            let (rem, vlan) = r;
            match vlan {
                EthernetTypeId::Vlan(vlan_type_id) => {
                    Ethernet::parse_with_existing_vlan_tag(rem, dst_mac, src_mac, vlan_type_id, agg)
                }
                not_vlan => {
                    do_parse!(rem,

                        payload: rest >>

                        (
                            Ethernet {
                                dst_mac: dst_mac,
                                src_mac: src_mac,
                                ether_type: not_vlan,
                                vlans: agg,
                                payload: payload.into()
                            }
                        )
                    )
                }
            }
        })
    }

    pub fn new(
        dst_mac: MacAddress,
        src_mac: MacAddress,
        ether_type: EthernetTypeId,
        vlans: std::vec::Vec<VlanTag>,
        payload: std::vec::Vec<u8>
    ) -> Ethernet {
        Ethernet {
            dst_mac,
            src_mac,
            ether_type,
            vlans,
            payload
        }
    }

    pub fn parse(input: &[u8]) -> nom::IResult<&[u8], Ethernet> {
        trace!("Available={}", input.len());

        let r = do_parse!(input,

            dst_mac: mac_address >>
            src_mac: mac_address >>

            ( (dst_mac, src_mac) )
        );

        r.and_then(|res| {
            let (rem, (dst_mac, src_mac)) = res;
            Ethernet::parse_vlan_tag(rem, dst_mac, src_mac, vec![])
        })
    }
}

impl TryFrom<Ethernet> for Layer2FlowInfo {
    type Error = errors::Error;

    fn try_from(value: Ethernet) -> Result<Self, Self::Error> {
        let ether_type = value.ether_type;
        debug!("Creating from layer 3 type {:?} using payload of {}B", ether_type, value.payload.len());
        let l3 = if let EthernetTypeId::L3(l3_id) = ether_type.clone() {
            match l3_id {
                Layer3Id::IPv4 => {
                    layer3::ipv4::IPv4::parse(&value.payload)
                        .map_err(|e| {
                            let err: Self::Error = e.into();
                            err.chain_err(|| errors::Error::from_kind(errors::ErrorKind::FlowParse))
                        }).and_then(|r| {
                        let (rem, l3) = r;
                        if rem.is_empty() {
                            Layer3FlowInfo::try_from(l3)
                        } else {
                            Err(errors::Error::from_kind(errors::ErrorKind::IncompleteParse(rem.len())))
                        }
                    })
                }
                Layer3Id::IPv6 => {
                    layer3::ipv6::IPv6::parse(&value.payload)
                        .map_err(|e| {
                            let err: Self::Error = e.into();
                            err.chain_err(|| errors::Error::from_kind(errors::ErrorKind::FlowParse))
                        }).and_then(|r| {
                        let (rem, l3) = r;
                        if rem.is_empty() {
                            Layer3FlowInfo::try_from(l3)
                        } else {
                            Err(errors::Error::from_kind(errors::ErrorKind::IncompleteParse(rem.len())))
                        }
                    })
                }
                _ => {
                    Err(errors::Error::from_kind(errors::ErrorKind::EthernetType(ether_type)))
                }
            }
        } else {
            Err(errors::Error::from_kind(errors::ErrorKind::EthernetType(ether_type)))
        }?;

        Ok(Layer2FlowInfo {
            src_mac: value.src_mac,
            dst_mac: value.dst_mac,
            vlan: Ethernet::vlans_to_vlan(&value.vlans),
            layer3: l3
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate env_logger;
    extern crate hex_slice;
    use self::hex_slice::AsHex;

    use super::*;

    const PAYLOAD_RAW_DATA: &'static [u8] = &[
        0x01u8, 0x02u8, 0x03u8, 0x04u8, 0x05u8, 0x06u8, //dst mac 01:02:03:04:05:06
        0xFFu8, 0xFEu8, 0xFDu8, 0xFCu8, 0xFBu8, 0xFAu8, //src mac FF:FE:FD:FC:FB:FA
        0x00u8, 0x04u8, //payload ethernet
        //payload
        0x01u8, 0x02u8, 0x03u8, 0x04u8
    ];

    const TCP_RAW_DATA: &'static [u8] = &[
        0x01u8, 0x02u8, 0x03u8, 0x04u8, 0x05u8, 0x06u8, //dst mac 01:02:03:04:05:06
        0xFFu8, 0xFEu8, 0xFDu8, 0xFCu8, 0xFBu8, 0xFAu8, //src mac FF:FE:FD:FC:FB:FA
        0x08u8, 0x00u8, //ipv4
        //ipv4
        0x45u8, //version and header length
        0x00u8, //tos
        0x00u8, 0x48u8, //length, 20 bytes for header, 52 bytes for ethernet
        0x00u8, 0x00u8, //id
        0x00u8, 0x00u8, //flags
        0x64u8, //ttl
        0x06u8, //protocol, tcp
        0x00u8, 0x00u8, //checksum
        0x01u8, 0x02u8, 0x03u8, 0x04u8, //src ip 1.2.3.4
        0x0Au8, 0x0Bu8, 0x0Cu8, 0x0Du8, //dst ip 10.11.12.13
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
    fn parse_ethernet_payload() {
        let _ = env_logger::try_init();

        let (rem, l2) = Ethernet::parse(PAYLOAD_RAW_DATA).expect("Could not parse");

        assert!(rem.is_empty());
        assert_eq!(l2.dst_mac().0, [0x01u8, 0x02u8, 0x03u8, 0x04u8, 0x05u8, 0x06u8]);
        assert_eq!(l2.src_mac().0, [0xFFu8, 0xFEu8, 0xFDu8, 0xFCu8, 0xFBu8, 0xFAu8]);
        assert!(l2.vlans().is_empty());

        let proto_correct = if let EthernetTypeId::PayloadLength(_) = l2.ether_type() {
            true
        } else {
            false
        };

        assert!(proto_correct);
    }

    #[test]
    fn parse_ethernet_tcp() {
        let _ = env_logger::try_init();

        let (rem, l2) = Ethernet::parse(TCP_RAW_DATA).expect("Could not parse");

        assert!(rem.is_empty());
        assert_eq!(l2.dst_mac().0, [0x01u8, 0x02u8, 0x03u8, 0x04u8, 0x05u8, 0x06u8]);
        assert_eq!(l2.src_mac().0, [0xFFu8, 0xFEu8, 0xFDu8, 0xFCu8, 0xFBu8, 0xFAu8]);
        assert!(l2.vlans().is_empty());

        let proto_correct = if let EthernetTypeId::L3(Layer3Id::IPv4) = l2.ether_type() {
            true
        } else {
            false
        };

        assert!(proto_correct);
    }

    #[test]
    fn convert_ethernet_tcp() {
        let _ = env_logger::try_init();

        let (rem, l2) = Ethernet::parse(TCP_RAW_DATA).expect("Could not parse");

        assert!(rem.is_empty());

        let info = Layer2FlowInfo::try_from(l2).expect("Could not convert to layer 2 flow info");

        assert_eq!(info.layer3.layer4.src_port, 50871);
        assert_eq!(info.layer3.layer4.dst_port, 80);
    }

    #[test]
    fn test_single_vlan() {
        //TODO
    }

    #[test]
    fn test_multiple_vlans() {
        //TODO
    }
}